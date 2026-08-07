# Architecture: Engine Logging & Diagnostics (`boyko_log`)

> Target file: `docs/LOGGING-SYSTEM-PLAN.md`. Status: **DRAFT v4 — revised against the third-pass review (verdict REJECTED, 10 blockers B1-B10 + 4 majors M1-M4) AND the approved seam decision record for `boyko_diag` (S1-S12), which judged the v3 seam INCOMPATIBLE AS WRITTEN.** Findings disposition, seam disposition and scope-extension disposition at the end.

## Changelog v3 → v4

v4 folds **two** inputs. They stay separable: §Findings disposition (v3 → v4) is the defect half, §Seam disposition (S1-S12) is the cross-plan half. Where they collide the collision is named at the point of collision, not in a table.

**Input 1 — the third-pass review (REJECTED, 10 blockers + 4 majors).** Five of them are outright design holes, fixed by structure and not by prose:

- **B1** — `LogRing`/`LogCensus` hold a `VmColumn`, and `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70` states verbatim that `VmColumn` is **NOT `Send`/`Sync`**, while `crates/boyko_ecs/src/ecs/core/resources/resource.rs:42` requires `Resource: 'static + Send + Sync + Sized`. v3's fold therefore did not compile — F7's failure mode one level up. v4 writes the SEND10-shaped `unsafe impl` with the exclusivity argument naming every holder of `&`/`&mut`, and **removes the sink thread from that set entirely** (§Data structures, B1 block), pinned by a `const _` `assert_send_sync` check.
- **B2** — the sink→ECS "handoff ring" was referenced three times and defined nowhere. v4 specifies **`ECS_HANDOFF`** as a first-class SPSC byte ring reusing `LogLane`'s own wrap rule, with layout, capacity, ordering, overflow accounting (`W0117`), budget row and SAFETY clause (§Data structures, §Algorithms C). It is what makes B1's argument true: the sink writes `ECS_HANDOFF`, never `LogRing`.
- **B5** — the crash drain CAS'd `SINK_STATE` out of `{Exited, NotBooted, Manual}` and called all three quiescent. `Manual` is not: it means an arbitrary thread may be inside `drain()` **right now**. v4 CASes the **consumer role itself** — `DRAIN_OWNER`, claimed by the sink thread, by `drain()` and by the crash drainer alike (Decision 24) — and G14 gains the leg that panics with a manual drain in flight.
- **B8** — `shipping-min` was structurally guaranteed to contain the session's *beginning*: `Manual` never drains, admission control drops new records, so 32 × 16 KiB fills with boot records within seconds and everything up to the crash is refused. v4 gives the profile a real consumer: `log_drain_system` takes `DRAIN_OWNER` and runs the drain itself once per frame, on the frame thread, with the cost stated (Decision 10, Decision 25). The retained window is restated **in records**.
- **B10** — `LogPod`'s blanket `copy_nonoverlapping` of `size_of::<Self>()` copies padding bytes, and the sink then materialised a `&[u8]` over uninitialised memory — UB independent of whether `POD_LEN` was honest. v4 deletes the blanket copy: the trait requires `encode_pod`, the derive generates it **field-by-field** through `LogValue`, and `POD_LEN` is a const sum of field lengths rather than `size_of` (Decision 19b).

Three more were vacuous or misplaced gate legs — this campaign's recurring defect, now caught inside the very gates written to catch it. **B3**: G17's leg (c) could not go red, because the F6 overrun is intra-lane by construction and no neighbouring canary is reachable; it is replaced by an *undrained-record* assertion at a **second, explicitly named fill level**, because the two legs need different fills and v3 named one. **B4**: G4's sampling leg asserted 500 for a quantity that is 1000 (arguments are evaluated before the sample decision) and needed a mechanism that lands 12 rungs later; the observable is split and the leg moves to L12-gate as **G10e**, where the two numbers together are the claim. **B9**: three mechanisms promised durability and wrote stderr; `write_oracle_line` now fans out to **every configured synchronous destination**, including the boot-opened crash file, and the "durable-on-write" claim is restated against what a `write_all` actually guarantees.

**B6** and **B7** are corpus-and-walker defects measured directly against the tree; both fixes are stated in measured numbers rather than in shapes (Decision 6). **M1-M4** are folded: the per-code `fired=` figure becomes a real per-**site** observation through an intrusive `ONCE_SITES` list (and `RateSlot::fired` is deleted as dead); the integer audit gains `seq_lo`'s reconstruction rule and every `BinarySink` width; code-index exhaustion gets a defined, never-aliasing return; and the census's `vk-validation` showcase row — inert by construction, since Decision 9b guarantees no record ever reaches that target — is **dropped**.

**Input 2 — the seam decision record (`boyko_diag`).** A new zero-dep bottom crate owns the clock, lane identity, the loss vocabulary and the never-freed-storage policy; both this plan and `docs/PROFILING-SYSTEM-PLAN.md` consume it. The three edits that cost this plan the most:

- **S1** — **`report!` is deleted from this plan**, with test 16 and L8b's 20 measurement rows. The profiler owns the measurement channel end to end; its durable output is an artifact, never stdout. The migration denominator falls from ≤ 98 to **≤ 78**. `OUT_LOCK` **survives** — it still serves `write_oracle_line`, the sync-routed targets and `SINK_REQ` — so Decision 9c stays whole and G18 keeps its subject.
- **S3/S4** — lane identity and the clock both move to `boyko_diag`. This plan's `MAX_LANES`, its `hash(thread_id)` claim scan, its `Drop`-guard TLS, its retire protocol and `tsc.rs` are deleted; `W0101` is struck. The honest consequence, recorded where the number lives and not only here: deleting the `Drop` guard takes "**≤ 1 allocation on a thread's first emit**" to **0**, so the row that existed to be honest about a cost now records that the cost is gone.
- **S9** — one compile axis, `BOYKO_PROFILE`, owned by `boyko_diag/build.rs`. **`crates/boyko_log/build.rs` is not created.** Decision 25's table loses its `GLOBAL_CEILING` column (a *runtime* preset cannot deliver a *compile-time* const) and the preset is renamed **`LogRuntimePreset`**; the header prints `build_profile` / `runtime_preset` / `ceiling` as three independent facts.

**The joint cost is absorbed, not deferred.** "Producer working set ≤ 4 cache lines" is true **in isolation only**; with the profiler armed the joint figure is **7-8**, and that sentence now sits next to the number in §Performance targets, not only in a disposition table.

**What did NOT change:** deferred formatting, the POD record + `&'static LogSite`, `.bss` statics with `Off == 0`, the SPSC lane ring itself, B1's staged-copy drain, B3's shared wrap rule, B8's withdrawal of the validation migration, M23's tidy-test-primary enforcement, Decision 9c's `OUT_LOCK` protocol, and every v3 fold the third-pass review did not name.

## Changelog v2 → v3

v3 folds **two** inputs at once. They are kept separable on purpose: a reader who only cares about the defects can read §Findings disposition (v2 → v3), and a reader who only cares about the new audience can read §Scope-extension disposition. Where the two collide, the collision is named in §Audience conflicts and decided there.

**Input 1 — the second-pass review (REJECTED, 26 findings).** Two of them are outright correctness bugs and are fixed by arithmetic and by a type size, not by prose:

- **F6** — `free = limit - (w - read_cached)` **underflows in `u32`** exactly in the state the Error reserve is designed to produce, and the producer then overruns live ring bytes. Fixed by reformulating admission control as `avail = CAPACITY - used` (an induction that cannot go negative) and applying the reserve with `saturating_sub` (§Decision 5, §Algorithms A6). Gate **G17** exists solely to red on the old arithmetic.
- **F7** — `LogRing`'s `VmColumn<LogLine>` **panics at construction**: `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` asserts `COMMIT_GRANULE % size_of::<T>() == 0`, `COMMIT_GRANULE = 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`), and v2's `LogLine` was 12 bytes. `LogLine` is now **16 bytes, `Copy`, with a `const _: () = assert!` beside the definition** so a future field addition fails the build instead of the plugin (§Data structures).

Four more were "a gate that cannot fail" in a new costume — the campaign's own recurring defect — and each is either given a showable red state or **deleted**: test 17 (F1), G7's negative leg (F2), P1 (F3), G2's thread/hook legs (F4), registry checks 3/6 (F5). `OUT_LOCK` grows a **bounded, re-entrancy-aware, unwind-safe** protocol (F8) and is **not** registered in `docs/HOT-PATH-EXCEPTIONS.md`, because registering it **reds CI** (F9 — `scripts/check_hotpath_exceptions.py:337-341` matches registry rows against `#[allow(clippy::disallowed_types)]` counts per file, and an atomic carries none).

**Two findings are REFUTED with evidence, and one is refuted in part** — see §Findings disposition. v2 said "Refuted: none"; that was a disposition, not a virtue, and it is not repeated for its own sake.

**Input 2 — the scope extension (games, not just the engine).** Dynamic targets minted from data (Decision 18), downstream code tables (Decision 19), game-defined POD values (Decision 19b), per-target sampling (Decision 20), a session-scale integer audit (Decision 21), a binary sink that does not format (Decision 22), runtime sink/level control with no restart and no lock (Decision 23), a crash drain (Decision 24), four shipping profiles (Decision 25), and the in-frame reader surface a `boyko_ui` HUD and a telemetry reducer consume (Decision 17, Decision 26). Seven asks are **refused with reasons** rather than designed (§Refused).

**What did NOT change, because the review said to keep it:** deferred formatting, the POD record + `&'static LogSite`, `.bss` statics with `Off == 0`, the SPSC lane, `report!` as a separate synchronous channel, B1's staged-copy drain, B3's shared wrap rule, B8's withdrawal of the validation migration, N30's honesty about `W0101`, and M23's tidy-test-primary enforcement.

### Answers to the first review's eight questions (carried forward; two amended by v4, marked inline)

1. **Drain order** — the drain copies each record out of the ring into a staging arena, *then* advances `read`, *then* sorts, *then* decodes from staging. `read` never advances over bytes the sink still intends to read. (§Algorithms C)
2. **Encoded length** — `LogArgs::encoded_len(&self) -> usize`, a runtime method that const-folds for all-fixed tuples. `&str` encodes as `u16` length + bytes. `fmtv` no longer exists; a `Display` is rendered by `dsp!` into a caller-owned stack buffer **in argument position**, so the ring is never open while user code runs, and an overrun is a truncation of a `&str` that has already been produced. (§Decision 1a, §Decision 13)
3. **Wrap** — records never straddle. Deterministic shared rule: `LANE_BYTES - off < HEADER_BYTES ⇒ both sides wrap`; otherwise a PAD record (null `site`, `len` = tail) consumes the tail. (§Algorithms A6)
4. **Re-entrancy** — forbidden *structurally*: nothing between lane acquisition and the `Release` store can call user code, because argument encoding is over already-materialised POD and `&str`, and `LogPod::fmt_pod` runs on the sink. A re-entrancy `debug_assert` guard backs it; test 24 asserts it. (§Decision 13, §Decision 19b)
5. **Sink rate** — stated as a design number (≥ 500 K records·s⁻¹ **aggregate**, text sink, at the default geometry) and **gated at L3** by `sink_sustained_rate`, which must show the drop knee. (§Decision 10, §Metrics)
6. **Perturbation** — ABBA-counterbalanced logger-on/logger-off with an interleaved zero control in the same sitting. **v3 changed the instrument** (headless schedule bench, not windowed frame time, which FIFO clamps); **v4 changes the leg matrix**: P1 is a 2×2 of {logger off, on} × {profiler absent, armed}, because a perturbation measured with the other subsystem in an unstated state is not a measurement of either (§Metrics, gate P1, F3, S10).
7. **`[vk-validation]`** — stays synchronous. The migration is withdrawn, **and so is v2's remaining one-line edit** (§Decision 9b, F12). **v4 additionally deletes the census's `vk-validation` row**, which was inert by construction (M4).
8. **Handle / flush-without-consumer** — `LogHandle` is deleted; `boot()`/`shutdown()` are free functions over process-lifetime statics. `flush()` reads `SINK_STATE` first and returns `FlushResult::NoConsumer` immediately when nothing can ever acknowledge. There is no `join`-with-timeout because std does not provide one; shutdown observes a sink-exited atomic with a bounded spin and then **detaches**. (§Decision 12)

---

## Goal

Replace the workspace's ad-hoc diagnostic output — **179 raw occurrences across 36 files** under `crates/*/src/**` (reproduced this session: `grep -rEn 'println!|eprintln!|print!\(|eprint!\(' --include='*.rs' crates/*/src` ⇒ 179 / 36; `crates/boyko_shaderdsl/src/bin` ⇒ 58) — 5 hand-rolled `AtomicBool` once-latches, and 9 ad-hoc `boyko-####` codes in three incompatible text formats, with **one in-house subsystem** that obeys the engine's own principles.

**The census arithmetic, stated so it cannot drift again** *(fixes F18)*. v2 opened with "83 shipping print sites". Its own ledger says 179 − 58 (CLI bins) − 23 (in-`src` `#[cfg(test)]`) = **98**. Neither number is a *site* count: 179 is a **raw occurrence** count over unstripped text, and it includes prose — `crates/boyko_app/src/runner.rs:560-561` mentions `eprintln!` twice inside a comment. Two numbers are therefore carried, both named:

- **179 / 36** — the reproducible raw-grep census. Its only job is to be re-derivable by one command.
- **The walker's site count** — macro invocations after comments, string literals and `#[cfg(test)]` regions are stripped (§Decision 6's walker). It is ≤ 98 and it is the migration denominator. **L8c's exit criterion is defined over the walker's count, never over the grep's**, so a comment mentioning `eprintln!` can never be driven into `print_allowlist.txt` to make a gate go green.

**Two audiences.** The engine is audience one: a fixed set of subsystems, known at compile time, where the right answer to "should this site cost anything when disabled?" is *nothing at all, not even a load*. A **game built on this engine** is audience two: hours-long sessions, categories that may be named by data or by a mod, a console that toggles verbosity without a restart, a support-facing log, and gameplay code that wants to read its own diagnostics in-frame. The two are not the same customer, and §Audience conflicts names the five places where they pull in opposite directions and decides each with the cost to the losing side.

**Functional**
- Any thread — Chase-Lev worker, dispatcher, OS/window thread, asset I/O, a script VM, a mod — emits a diagnostic without a lock, an allocation, or a syscall.
- Every `Warn`/`Error` carries a registry code that is documented, uniquely numbered, mechanically proven non-orphan, and explainable. This holds for game and mod code too (Decision 19); there is no relaxed tier.
- **Loss is counted and reported; never silent — and *policy* is reported as policy, not as zero** *(fixes F10; the reporting mechanism corrected by M1)*. Three quantities are kept apart on purpose, because folding any two of them makes one of them a liar: `dropped` (the ring refused it — a loss, `boyko_diag::LossClass::Overflow`), `sampled_out` (a declared 1-in-2^k policy skipped it — not a loss), and `suppressed` (a rate policy skipped it — not a loss). `Once` deliberately does **not** count its suppressions, because counting them costs a per-occurrence RMW on a shared line, which is the exact defect this campaign found in the hand-rolled latches. v3 answered that with a census line reading `fired=1`, which was a **literal, not an observation** — with the latch per-site, nothing aggregated per code, and the count was wrong in exactly the three-site `W2102` case F11 was raised for. v4 replaces it with a real observation: a `Once` site pushes itself onto an intrusive `ONCE_SITES` list on its **single** fire (`#[cold]`, once per site per process, zero steady-state cost), and the census prints **one row per fired site** — `code=W2102 site=device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)`. A code whose suppressed count genuinely matters declares `OnceCounted` and pays the RMW at its own declaration site, and its row carries a real number.
- **Evidence channels are synchronous.** Validation-layer messages do not travel on the async path (Decision 9b). **Measurement lines are no longer this crate's business at all**: S1 gives the measurement channel to the profiler end to end, and `report!` is deleted (Decision 9).
- The **absence** of records on an armed target is reported as `UNPROVEN`, never as `clean` — and the extension adds two further ways to manufacture that silence, each of which gets its own status (§Decision 17: `UNPROVEN(sampled)`, `UNPROVEN(unsunk)`). *(v3 listed a third, `dropped=SATURATED`; S8 widens the counters to `u64` and the saturation token is struck — a counter that cannot saturate needs no token for the state.)*

**Performance targets** — every row has a control measured in the same sitting; a number without a control is not a measurement, and this repository has measured its own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol.

| Metric | Target | Control that can go red |
|---|---|---|
| Compile-disabled site | no `emit_impl` symbol reference in the object file | the armed variant of the same fixture **must** show the symbol (§Metrics G1, **at L1-gate — F19**) |
| Runtime-disabled site | ≤ 3 ns | enabled variant of the same site, same bench |
| **Runtime-disabled `warn!`/`error!` with no consumer** | **≤ 4 ns** — S5's `sink_can_accept()` adds one `Relaxed` load and one predicted-not-taken branch to the **failed-gate** path of `warn!`/`error!` only, so that a severe record before `boot()` or after `shutdown()` takes the synchronous channel instead of vanishing | `log_disabled_runtime` (`info!`, untouched by the predicate) in the same sitting; if the two resolve apart by more than the branch, the predicate is moved behind the level test |
| Enabled, 0 args | ≤ 15 ns median | runtime-disabled, same sitting |
| Enabled, 2×u32 | ≤ 20 ns median | as above |
| Allocations on the producer path, **steady state** | **0**, proven by a **per-thread** counting allocator | armed sink thread must show > 0 on its own thread |
| Allocations on a thread's **first** emit | **0** *(was ≤ 1 in v3)*. The one allocation v3 admitted was the `thread_local!` destructor registration for **this plan's own lane guard**; S3 deletes that guard — lane identity is `boyko_diag::lane()`, a `Cell<u16>` TLS with **no `Drop`** — so the allowance has no remaining source | test 3 leg (b) asserts **`== 0`** on a fresh thread. Reinstate a `Drop`-carrying TLS guard ⇒ the count becomes 1 ⇒ red (S3 red (c)) |
| Syscalls per record | **0**; one `write` per drain per byte sink | — |
| Producer working set | **≤ 4 cache lines in isolation — 7-8 jointly.** The 4-line figure is this crate measured alone. With the profiler armed the same producer also touches `ARM_MASK`, the `ZoneLane` control line and the sample tail, and the joint working set is **7-8 distinct lines** (seam decision record, §Joint cost). The isolated number is not wrong; it is **not the number a shipped frame pays**, and it must never be quoted without the joint one | the P1 2×2 matrix (S10) reports both states; a working-set claim taken at one profiler state and reported as unconditional is the defect this row exists to prevent |
| Sustained rate before drop | ≥ 500 K records·s⁻¹ aggregate, text sink, at default geometry | `sink_sustained_rate` must find the drop knee |
| Sustained rate, **binary sink** | ≥ 5× the text sink in the same sitting | **if it does not separate, L13b is reverted** (G12c) |
| CPU frame-work perturbation, logger idle | not resolvable above the sitting's floor, **on a channel that can respond, at a stated profiler state** | headless schedule bench + interleaved zero control, **2×2 over {profiler absent, armed}** (gate P1, re-specified — F3, S10) |
| Resident memory | `claimed_lanes × 16 KiB` + the fixed table budget (§Decision 3's matrix); `LOG_LANES` in `.bss`, gated. This crate reserves **≈ 2.90 MiB `dev` / ≈ 1.15 MiB `shipping``**. **Jointly with the profiler: ≈ 9.58 MiB `dev`, ≈ 1.95 MiB retail** — arithmetic in Decision 3, including why the seam record's 9.33 MiB is one revision old. The shared substrate saves 0.78 MiB in `dev` and **nothing** in `shipping`; it is bought for correctness, not footprint | section gate G3 (`boyko_diag::section_report`) |
| Fully-off build | `size_of_val(&LANES) == 0`, no sink thread, no panic hook — **each with a named observation mechanism** | build leg G2 (F4), now `BOYKO_PROFILE=off` (S9) |
| Session-scale honesty | no counter wraps, no capture silently truncates | G11 (**u64 fold exactness**, S8), G12b (rotation), P2 (30-min soak) |

Published reference band: Quill 8-9 ns, NanoLog 7 ns median (both vendor-published, deferred-format); spdlog 242 ns (caller-side format). The ~30× gap **is** caller-side formatting. That single fact organises this design.

---

## Context and constraints

### Affected subsystems *(re-cut by S2, S3)*
New crate `boyko_log`, whose `[dependencies]` are exactly **`boyko_diag` and `boyko_macros`** — no third-party, no other workspace crate. `boyko_diag` is the shared diagnostics substrate below it (`std` only, out-degree 0), owned jointly with `docs/PROFILING-SYSTEM-PLAN.md`; `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md` is its own document.

`boyko_threadpool`, `boyko_ecs`, `boyko_rhi`, `boyko_rhi_vulkan`, `boyko_image`, `boyko_physics`, `boyko_serialize`, `boyko_render`, `boyko_ui`, `boyko_app`, `boyko_demo` gain a `-> boyko_log` edge. **`boyko_utils` does NOT** *(S2)*: v3 wrote "`boyko_utils` … depend on it", but nothing in `boyko_utils` logs, and `boyko_utils` is a zero-dep leaf the seam record keeps zero-dep on purpose. The one thing v3 cited that edge for — "`TypeIntern` is unusable for `TargetId`" — survives unchanged for the unrelated reason already recorded in Decision 15 (`ID` must be a `const`), so striking the edge costs nothing.

**Acyclicity.** `boyko_diag` has out-degree 0; `boyko_log`'s only out-edges are `boyko_diag` and `boyko_macros` (whose own edges are `syn`/`quote`, both external). No workspace crate is reachable from either, so no crate depending on `boyko_log` can be reached by it.

**Lane identity is minted by `boyko_diag`, not by `boyko_log`** *(S3)*. `boyko_diag::lane()` is a `#[inline]` read of a `Cell<u16>` TLS with **no `Drop`**; `boyko_threadpool` sets it at `worker_main` (`crates/boyko_threadpool/src/worker.rs:21`, whose `worker_id: u32` is dense by construction) and at `ThreadPool::install` entry/exit; `boyko_app`'s runner boot takes `LANE_HOST`; anything else calls `boyko_diag::claim_lane()`. `LANE_WORKER_MAX = 64` matches `crates/boyko_threadpool/src/thread_pool.rs:49`'s `pub const MAX_WORKERS: usize = 64`, verified. This still keeps the threadpool able to log — the substrate is below the threadpool, not above it — and it buys the property no separate registry can: **a log record and a profiler zone on the same worker carry the same integer**, which is the only reason a reader can place a line inside the zone it happened in.

Two file-level consequences outside this crate, recorded so they are not lost: `crates/boyko_image/Cargo.toml:5`'s description claims "no dependency on any other workspace crate" and becomes false at L8a; `crates/boyko_rhi_vulkan/Cargo.toml:42-49`'s in-file no-third-party rationale gains a `boyko_log`/`boyko_diag` row on the `boyko_sdf_math` precedent already written there.

### Invariants preserved
1. `clippy.toml`'s `disallowed-types` — no `HashMap`/`HashSet`/`Mutex`/`RwLock`/`Rc`/`RefCell` in this crate at all. **`OUT_LOCK` is NOT a registered hot-path exception, and must not be made one** *(fixes F9; and `OUT_LOCK` **survives S1** — deleting `report!` does not delete the lock, because `write_oracle_line`, the sync-routed targets and `SINK_REQ` are all still callers, so Decision 9c and G18 stand unchanged)*. Read against the tree: `scripts/check_hotpath_exceptions.py:15-19` requires "the row count per file must match the allow count per file", `:51` counts only `#[allow|expect(clippy::disallowed_types)]` sites, and `:337-341` fails a file whose registry rows exceed its allow count with "registry lists N exception(s) but the file has none left". `OUT_LOCK` is an `AtomicU64`, which the ban does not cover, so `sync_out.rs` carries no `#[allow]` and a row for it would be **drift ⇒ exit 1**. The registry exists for the *type ban*, not for locks in general. What `OUT_LOCK` needs instead is a **protocol that cannot hang** — §Decision 9c — and a mechanical gate over that protocol (G18), not a paragraph in a file that would reject it.
2. **The stdout/stderr machine API — inventoried, not estimated** *(fixes F13, F14)*. Measured this session:
   - `[vk-validation]` is referenced by **31 files**: the producer `crates/boyko_rhi_vulkan/src/debug.rs:114`, `scripts/golden.ps1`, and 29 tests/examples. `golden.ps1:196-202` does **not** do a plain `2>&1` — it routes through `cmd /c … > "$valLog" 2>&1` because PS 5.1 wraps native stderr into `NativeCommandError` records, and `:226` then `Select-String`s the merged file, printing `VALIDATION: clean (0 messages)` in green at zero (`:232`). `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118` pins the prefix **byte-exact including the trailing space** (`"[vk-validation] "`) and its own comment calls it "the gate's entire input".
   - `VB-P1d` is referenced by **16 files** (3 producers under `src/`, 13 consumers).
   - **Correction to v2's M24 claim.** v2 wrote that `vg_occ_split_timing.rs` "parses that merged stream". It does not: `:1115-1117` uses `cmd.output()` and concatenates `out.stdout` **then** `out.stderr` in Rust — two separately buffered pipes, between which interleaving is structurally impossible. The `OUT_LOCK` justification v2 attributed to that consumer was fictional. The real merged-stream consumer is `golden.ps1`, and the real ordering hazard is *within* stdout (F17, §Decision 9).
   - **All of these contracts survive byte-for-byte and remain synchronous** (Decisions 9, 9b). **Nothing in this plan writes to stdout at all** — and after S1 nothing in it writes the `VB-P1d`/`VB-P4` lines either: those six consumers are migrated to the profiler's artifact at profiling rung 7, before L8b, by `docs/PROFILING-SYSTEM-PLAN.md`, not by this document.
   - **stdout and stderr, one rule each** *(S7)*. **stdout** is written by exactly one thing in the whole workspace: `boyko_shaderdsl`'s CLI bins (the 58 allowlisted sites). Nothing in the engine, the logger or the profiler writes stdout, ever. **stderr** is written by exactly two, and both go through `std::io::stderr()`'s **own handle**: the Vulkan validation messenger (`crates/boyko_rhi_vulkan/src/debug.rs:114`, untouched byte-for-byte) and `boyko_log::write_oracle_line`. Sharing the handle — rather than a raw fd — is what makes the two share stderr's inner lock, so **neither can splice a line into the other**. That is Decision 9's F17 lesson applied to stderr, and it is the reason `golden.ps1:226`'s line-start match on `[vk-validation] ` keeps working. *Ordering* between the two producers remains undefined and is stated as such; **line integrity**, not ordering, is what the gate consumes.
3. Existing `#[should_panic(expected = "boyko-B0002")]` assertions match on a substring, so normalising `error[boyko-B0002]: …` → `boyko-B0002: …` is safe.
4. Codes are never renumbered, never reused. `B9003` is a permanent gap.
5. `#[cold] + #[inline(never)]` on diagnostic helpers (`crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:1-6`).
6. **No new hang class.** `crates/boyko_app/tests/vb_bench_totality_gate.rs:48-49` records that this repository *has no kill-after-timeout pattern to borrow*. Every wait in this design is bounded and, where no acknowledgement is structurally possible, returns immediately with a reason. **v2 violated this invariant with its own `OUT_LOCK`** — an unbounded spin, no release-on-unwind, no re-entrancy story, and a `flush()` timeout path that terminated *in* that unbounded spin (F8). §Decision 9c is the fix, and G18 is its gate.
7. **Principle 0 — no longer an exception plea; an instance of a stated policy** *(fixes F16; restated by S12)*. v3 claimed a per-plan exception on dependency inversion. The seam record replaces the plea with **one rule that both diagnostics plans obey**:

   > **Extent known at compile time ⇒ `.bss` static. Extent chosen at run time from config ⇒ `VmReservation`, and the owner must therefore sit at or above `boyko_ecs`.**

   `LOG_LANES` (1.25 MiB reserved `.bss` in `dev`), `CONTROL`, `TARGETS`, `RATE`, `SAMPLE_CTR`, `TARGET_STATS`, `DYN_NAMES`, `SINKS`, `SITE_DICT`, `STAGE`, `SINK_OUT` and `ECS_HANDOFF` all have compile-time-const extents, so all of them are `.bss` — **by the rule, not by exemption**. Everything whose extent is a run-time quantity, or that the ECS can own, is a `Resource` on `VmReservation`-backed engine storage (`VmColumn`): `LogRing`, `LogStats`, `LogCensus`. Never a `Box<[u8]>`, never a `Vec`.

   The rule is not a softening. The owner's standing correction targets `std::Vec`/`Box` — heap, growable, not address-stable. `.bss` is none of those: demand-zero, address-stable, allocation-free, exactly like a reservation; the *only* property separating them is whether the extent is a run-time quantity. And the boundary is forced anyway: `VmReservation` is `pub(crate)` in `boyko_ecs` with a `libc` unix arm, so a std-only zero-dep `boyko_diag` cannot host it without minting a **second** per-OS memory backing against `vm.rs`'s single-source-of-truth clause — a worse Principle-0 breach than the one it would fix.

   The dependency-inversion argument still does work, but a narrower and more honest job: it is why `CONTROL` cannot be an ECS column at all (there is provably no `World` before `boot()`, inside a driver callback, or inside a panic hook), and the cost of that is stated rather than hidden — no `Query`, no change detection, no `EnableTag` over `CONTROL`; the mitigation is `control_epoch`, an `O(1)` repaint signal a UI polls instead of subscribing (§Decision 23).

   **Gated, not asserted**: `boyko_diag::section_report` is the **one** implementation of the `llvm-readobj`/`objdump` section probe for both plans (S12), so a toolchain change reds one gate rather than disagreeing across two.

### Constraints inherited from the audits
- `current_worker_id_or_dispatcher_lane()` maps `WORKER_ID_UNATTACHED` → lane `0` (`crates/boyko_threadpool/src/tls.rs:69-78`, read this session). Window thread, present thread, driver callback thread and test harness threads all land there. Reusing that router would make lane 0 MPSC. **`boyko_diag::lane` is the replacement router** *(S3)*, and it is the one this crate consumes: workers keep their dense id, `install` takes `LANE_DISPATCHER`, the host takes `LANE_HOST`, and every other thread `claim_lane()`s a spare — so no two live threads share a lane index and the SPSC property survives. v3 minted a second registry inside `boyko_log`; that is what made one worker "lane 5" to the profiler and "lane 37" to the logger.
- `ThreadLanePair<E>` (`crates/boyko_ecs/src/ecs/core/events/event_buffer.rs:63-140`) is the house layout precedent, including the manual `unsafe impl Sync` + SAFETY-block style.
- The counting-allocator gate is **process-global** and already produced an impossible negative delta from a sibling test thread; `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK`. A permanently resident sink thread cannot be serialised by a test-local lock (M18).
- `clippy.toml:21-25` states, from an empirical 2026-07 check, that **clippy silently ignores a config path it cannot resolve** — an unresolvable entry emits nothing. Any new clippy key is therefore a claim requiring a liveness proof (M23).

---

## Key decisions

### Decision 1: Deferred formatting — POD payload + `&'static LogSite` + monomorphised decoder
**What.** The call site writes a 20-byte packed header containing a `*const LogSite` and a POD encoding of the arguments. Formatting happens on the sink via `site.decode: unsafe fn(*const u8, usize, &mut LogFormatter)`, monomorphised per *argument-tuple type* (Rust dedups identical monomorphisations, giving Quill's `log_statement` sharing for free).

**Why.** spdlog is asynchronous and still costs 242 ns median because it formats on the caller; Quill/NanoLog cost 7-9 ns because they do not. `core::fmt::Arguments<'a>` borrows its temporaries and cannot outlive the call, so the C++ varargs-capture trick has no Rust analogue.

**Alternatives rejected.** *defmt linker-section interning* — depends on ELF section semantics and a linker script; this toolchain is windows-gnu / PE-COFF, and an 8-byte `&'static` pointer is already free and portable. *`tracing`* — `event!` emits a static callsite, an `Interest` atomic load and `__is_enabled()` even when disabled, then dispatches through `Box<dyn Layer>`; Bevy's own docs concede the runtime-filter cost. *Caller-side format into a stack buffer* — 100+ ns and re-imports `core::fmt` codegen into 83+ sites.

**Trade-off.** Arguments must be `LogValue` (POD + `&str`). A `Display` is rendered by `dsp!` at the call site, and that cost is **visible in the source**, which is the point.

### Decision 1a: Record length is a runtime quantity — `encoded_len(&self)`, not `const ENCODED_LEN` *(fixes B2)*
**What.**
```rust
pub trait LogValue: private::Sealed {
    /// Upper bound known at compile time; `usize::MAX` means "dynamic".
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;                 // folds to a const for POD
    unsafe fn encode(&self, dst: *mut u8) -> usize; // returns bytes written
}
pub trait LogArgs { fn encoded_len(&self) -> usize; unsafe fn encode(&self, dst: *mut u8) -> usize; }
```
For an all-POD tuple every `encoded_len` is a constant and LLVM folds the sum to an immediate — the const case loses nothing. `&str` encodes as `u16` length + bytes, capped at `MAX_STR_BYTES = 256` with the `STR_TRUNCATED` flag.

**Why.** v1's `const ENCODED_LEN` was undefined for the two implementors that produce large records, which meant the space reservation the entire overflow protocol, the Error reserve and `MAX_RECORD_BYTES` rest on was undefined exactly where it mattered.

**Trade-off.** One non-foldable add per dynamic argument. Measured by `log_enabled_str32` against `log_enabled_2u32`.

### Decision 2: Three gates, `&&`-chained — two const-folded, one byte load
```rust
if T::STATIC_CEILING as u8 >= LVL as u8            // const: per-target compile ceiling
    && $crate::GLOBAL_CEILING as u8 >= LVL as u8   // const IN boyko_log (see N27)
    && $crate::runtime_ceiling(T::ID) >= LVL as u8 // Relaxed u8 load
{ $crate::emit_impl(...) }
```
**Why.** The `&&` short-circuit is what guarantees arguments are never evaluated — `log`'s exact shape, and why its docs warn against side-effecting arguments. Unreal explicitly does *not* give this guarantee and documents `UE_LOG_ACTIVE` as the workaround. The per-target compile ceiling is Unreal's two-verbosity model, which `log`/`tracing` lack. The runtime gate is one `Relaxed` load from `static CONTROL: [AtomicU8; 256]` plus one `and` — puffin's measured ~1 ns `AtomicBool` shape, generalised (Decision 14).

**Alternatives rejected.** *Cargo features for the ceiling* — features are additive and unified; one crate enabling `max_level_trace` re-enables it for everyone. `option_env!` in a `const fn` has no such failure mode.

**Where the const comes from, after S9.** `GLOBAL_CEILING` is derived from **one** env var, `BOYKO_PROFILE`, read by **exactly one `build.rs` in the workspace — `crates/boyko_diag/build.rs`**, which sits at the bottom of the graph so a change to it rebuilds every dependent. `crates/boyko_log/build.rs` is **not created** (v3 planned one; S9 deletes it), and neither is `crates/boyko_ecs/build.rs`. Staleness is closed by `boyko_diag/build.rs` emitting `cargo:rerun-if-env-changed=BOYKO_PROFILE`. `boyko_log` re-exports the value as its own `pub const GLOBAL_CEILING`, so **N27's property is preserved verbatim**: the macro still writes `$crate::GLOBAL_CEILING`, the `option_env!`/`env!` is still never expanded into a caller crate where no rerun directive exists, and the const still folds at every site.

`BOYKO_LOG_MAX_LEVEL` survives **only** under `BOYKO_PROFILE=custom`; setting it while a named profile is selected is a `compile_error!` with a named message. That is the one axis rule S9 exists to enforce — two axes is how a binary ends up printing a ceiling its profile does not name.

**Trade-off.** Changing the profile rebuilds the workspace. `MAX_TARGETS = 256` is a hard cap.

### Decision 3: Statics in `.bss`; `Off == 0`; a genuinely-off build *(extends v1, fixes M21; re-specified by F4, F21, F25; lane extent re-sourced by S3/S9)*
**What.** `LANE_BYTES = 16 KiB`. The **lane count is no longer this crate's constant**: it is `boyko_diag::LANE_COUNT`, **80** in `dev`/`editor` and **32** in `shipping`/`shipping-min`, selected by `BOYKO_PROFILE` (S3, S9). v3's `MAX_LANES = 128` is deleted. 80 is a *max*, not a sum: 64 workers (a hard const matching `thread_pool.rs:49`) + dispatcher + host + 14 claimable spares, which is ~7× the measured non-pool thread count in this engine. The lane array is a static, sized by that `const`:
```rust
pub const LANE_ARRAY_LEN: usize =
    if (GLOBAL_CEILING as u8) == 0 { 0 } else { boyko_diag::LANE_COUNT as usize };
static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = [LogLane::NEW; LANE_ARRAY_LEN];
```
`boot()` is a no-op when `GLOBAL_CEILING == Off`: no sink thread, no panic hook, no `RATE` traffic.

**What the smaller array costs, stated.** The claim scan no longer spreads by `hash(thread_id)`: `boyko_diag::claim_lane()` is a load-then-CAS over the 14 spares in index order, so concurrent claimants can convoy on the first free slot — bounded at 14 CAS attempts on a `#[cold]` path taken **once per thread**. A thread that never calls `release_lane()` holds its spare for the process: bounded at 14 × `LANE_BYTES` = 224 KiB, counted as `lanes_leaked` and printed in the census.

**Why — four properties, each load-bearing.**
1. No boot check on the hot path; a heap block behind an `AtomicPtr` would cost an `Acquire` load plus a null branch per record and create a "not booted" state at every site.
2. The unbooted default is correct and free: `.bss` is zero, `Level::Off == 0`, so every target reads `Off`, the gate fails, nothing happens.
3. Demand-zero paging: resident cost is `claimed_lanes × 16 KiB`, typically 8-12 lanes ≈ 128-192 KiB.
4. **`BOYKO_PROFILE=off` is a real off switch**, not merely a site-folding switch: zero lanes, zero threads, zero hooks. *(v3 named `BOYKO_LOG_MAX_LEVEL=off`; S9 makes `BOYKO_PROFILE` the single axis and that spelling survives only under `custom`.)*

**Gated, not assumed** (M21): `.bss` residency of a `MaybeUninit` static on PE/COFF is a toolchain behaviour, so gate **G3** asserts the section owning `LOG_LANES` carries a size with no raw data. The probe itself is **`boyko_diag::section_report`** (S12) — one implementation shared with the profiling plan's G22, so a PE/COFF toolchain change reds one gate instead of splitting two.

**G2 is re-specified: each of its three legs names its observation mechanism** *(fixes F4)*. v2's G2 asserted "`size_of_val(&LANES) == 0` and that no sink thread is spawned" and Decision 3 added "no panic hook", with no mechanism for either non-size leg — so `boot()` could spawn the thread and install the hook while `LANE_ARRAY_LEN` was 0 and G2 would still be green.

| Leg | Mechanism | Named red state |
|---|---|---|
| (a) size | `const _: () = assert!(LANE_ARRAY_LEN == 0)` in the off leg. **This is a const tautology and is kept only as env-plumbing proof** — it reds when `BOYKO_PROFILE=off` fails to reach the crate **through `boyko_diag`'s `build.rs`**, which is now a two-crate path and therefore a *more* useful plumbing check than v3's one-crate version. It is annotated as such in the test so nobody mistakes it for the claim | unset the CI leg's env var |
| (b) no sink thread | **OS thread count across `boot()`**: Windows `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` counting threads whose `th32OwnerProcessID == GetCurrentProcessId()`; Linux `std::fs::read_dir("/proc/self/task").count()`. Test-only, ~15 lines, one `#[cfg]` pair. **Its own control**: the same fixture spawns one deliberate `std::thread` and asserts the probe's count rises by exactly 1 — so a probe that always returns a constant reds before it can certify anything | make `boot()` spawn unconditionally ⇒ count rises ⇒ leg (b) reds |
| (c) no panic hook | **Behavioural, not identity-based** — `std::panic::take_hook()` is destructive and returns an unidentifiable `Box<dyn Fn>`. The fixture installs its **own** probe hook before `boot()`, then panics under `catch_unwind`, and asserts (i) the probe fired **exactly once** and (ii) the captured stderr contains **no** `boyko-log` marker line | make `boot()` chain its hook unconditionally ⇒ the marker appears ⇒ leg (c) reds |

**What G2 cannot claim**: that the off build has *no* cost. It has one: the crate is still compiled, linked and depended upon by every other crate. G2 bounds the *runtime* footprint to zero lanes, zero threads and zero hooks; it says nothing about compile time or binary size, and the `shipping` profile (Decision 25), not `off`, is the configuration a real title ships.

**`.bss` budget matrix, stated in full** *(fixes F25 — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box` in any signature" is carefully narrower than the Principle-1 claim a reader takes from it)*. Every one of these is a `.bss` static, demand-zero, never heap:

| Table | `dev` | `shipping` | Note |
|---|---|---|---|
| `LOG_LANES` | **80** × 16 KiB = **1.25 MiB** | 32 × 16 KiB = 512 KiB | reserved; resident is `claimed_lanes × 16 KiB`. **80/32 from `boyko_diag::LANE_COUNT`** (S3) — v3's 128 saved 768 KiB by going away |
| `RATE` | 512 × 64 B = 32 KiB | same | per-code; `Once`/`OnceCounted` no longer use it at all (§Decision 8) |
| `SAMPLE_CTR` | `LANE_COUNT` × 256 × 2 B = **40 KiB** | 16 KiB | one row per lane, producer-private (v3: 64 KiB at 128 lanes) |
| `TARGET_STATS` | 256 × 64 B = 16 KiB | same | consumer-written |
| `CONTROL` + `TARGETS` + `DYN_NAMES` | 256 B + 2 KiB + 2 KiB | same | |
| `ONCE_SITES` list heads | 8 B | same | one `AtomicPtr`; the nodes are the per-site statics themselves (M1) |
| `STAGE` | 256 KiB | 256 KiB | **`static STAGE: UnsafeCell<[u8; STAGE_BYTES]>`** — consumer-owned, reused every drain, never allocated |
| **`ECS_HANDOFF`** | **256 KiB** | **64 KiB** | *(new — B2)* the sink→ECS SPSC byte ring; present only when `ecs_ring` is enabled. Absent in `shipping-min` (no ECS reader) |
| `SITE_DICT` + `SINK_OUT` | 64 KiB + 1 MiB | 64 KiB + 256 KiB | binary sink only; absent unless `BinarySink` is configured |
| **Total reserved** | **≈ 2.90 MiB** (2 972 KiB) | **≈ 1.15 MiB** (1 180 KiB) | resident is a small fraction of each |

**Joint footprint, because the isolated one is not what a frame pays.** With the profiler present the `dev` total is **≈ 9.58 MiB** and the retail total **≈ 1.95 MiB**. **Arithmetic, so a reader can check it rather than trust it**: the seam record's joint table computes `dev` as 9.33 MiB from a logging `.bss` of 2.63 MiB — v3's 3.40 MiB less S3's lane cut (128 → 80 lanes, −768 KiB) and `SAMPLE_CTR` (64 → 40 KiB, −24 KiB). That figure **predates `ECS_HANDOFF`**, which B2 adds in this revision at 256 KiB in `dev`, so the joint `dev` total is 9.33 + 0.25 = **9.58 MiB**. The retail figure is unchanged at 1.95 MiB, because `ECS_HANDOFF` is 64 KiB there and the `.bss` shipping row already rounds to 1.15/1.21. **The seam record's table is not wrong; it is one revision old on one row**, and this is where that shows.

The shared substrate saves **0.78 MiB in `dev` and 0 MiB in `shipping`** — it is bought for correctness (one lane number, one clock epoch, a loss report that cannot itself be dropped), **not** for footprint, and this document does not claim otherwise. The retail figure against the profiling plan's "≤ 1 MiB retail" headline is an owner-facing VALUES question recorded in §Open questions.

**Honest floor when "on"**: the matrix above, one OS thread (except `shipping-min`), one process-global panic hook, one `VmReservation`-backed `LogRing` when the ECS seam is enabled, and a mandatory dependency edge from every crate onto `boyko_log` **and** transitively onto `boyko_diag`. That is the cost of the system existing; it is stated, not smoothed.

**Off-build dead code** *(fixes F21; simplified by S3)*: v2 set `LANE_ARRAY_LEN = 0` while §Algorithms B scanned `start..start+MAX_LANES` and indexed `LANES[i]` — dead but panicking code that G2 could not distinguish. **After S3 there is no claim scan in this crate at all**: the index comes from `boyko_diag::lane()`, and the only array access is `LOG_LANES.get(id as usize)`, which yields `None` on a zero-length array and takes the exhaustion path. In the off build no call site survives the const gate anyway, including the `Warn`/`Error` fallback, because `GLOBAL_CEILING == Off` deletes every level.

### Decision 4: SPSC byte ring per lane — the ring is ours, the **identity is `boyko_diag`'s** *(re-cut by S3)*
**What.** `LOG_LANES[i]` is a single-producer/single-consumer byte ring, indexed by `boyko_diag::lane()`. **This crate no longer mints, claims, retires or reclaims lane identity.** v3's `MAX_LANES`, its `hash(thread_id) % MAX_LANES` claim scan, its `Lane::owner` CAS, its `RETIRING` protocol and its `Drop`-carrying TLS guard are all **deleted** and replaced by `boyko_diag::lane` (A2): `lane()`, `set_lane()`, `claim_lane()`, `release_lane()`.

**Why the identity had to move, and not merely be shared.** Two registries mean two lane numbers for one thread: the same worker is lane 5 to the profiler and lane 37 to the logger, and no reader can then place a log line inside the zone it happened in — the one joint question the two subsystems exist to answer becomes unanswerable **by construction**, not by a bug. Separately, v3's `Drop` guard was exactly the `thread_local!`-destructor mechanism the profiling plan deliberately refused, **and** it was the sole source of this plan's "≤ 1 allocation on a thread's first emit" row. Deleting it takes that row to **0**.

**Why the ring stays ours.** True SPSC is why the threadpool's *router* is not reused: `current_worker_id_or_dispatcher_lane()` maps every non-pool thread to lane 0. `boyko_diag::lane` fixes the router without touching the ring, and the ring's shape is a logging decision:
- The producer caches the opposite cursor. The one published measurement on this question found padding **alone** made a ring *slower* — both threads still read the opposite cursor every operation — and only opposite-cursor caching *plus* padding moved throughput from ~32 to ~440 M ops·s⁻¹. We do both and treat padding as a hypothesis with an ablation bench, matching this repo's own `reference-componentpool-cache-stagger` lesson.
- Records are POD with no `Drop` and the array is a `static` that never moves, so a retired-undrained lane leaks nothing — which is why reclaim can be lazy and consumer-driven.

**Retire/reclaim, restated against the new owner.** `boyko_diag::release_lane()` marks the substrate's slot `RETIRING`; the consumer, per drain, reads `boyko_diag::lane_state(i)` and calls `boyko_diag::reclaim(i)` only after observing `RETIRING && read == write` for `LOG_LANES[i]`. The ordering argument is unchanged — the producer's last write precedes `release_lane()`, and the reclaim follows a drain to `write` — but the **state now lives in one place**, so the profiler cannot hand the same index to a new thread while this crate still believes it is live.

**`load`-then-CAS survives, in `boyko_diag`** (M10): the claim path is `if slot.load(Relaxed) == FREE { try CAS }` over the 14 spares. An unconditional `compare_exchange` over the array takes every occupied slot's line exclusive — the exact defect this repo already fixed at `crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:36-51` ("load first, store once"). The `hash(thread_id)` spread is gone with the scan; its cost is bounded and stated in Decision 3.

**Alternatives rejected.** *Double-buffer + wholesale swap* (`EventBuffer::swap_and_flatten`) — needs a quiescence point that boot code, the present thread and a driver callback do not have. *One MPMC ring* — CAS on every push, reintroducing the contention the per-lane design removes by construction. *Keeping a private lane registry and mapping it to `boyko_diag`'s at read time* — a mapping table is a second source of truth for one datum, which is Decision 14's `LogFilter` defect in a new place.

### Decision 5: Overflow drops, counts, and reports — corrected arithmetic, saturating counters, one aggregated report *(fixes F6, F24; extended by X4)*

**The defect first, because it is the reason this decision was rewritten.** v2's admission control read:

```
limit = LANE_BYTES - if level == Error { 0 } else { ERROR_RESERVE }   // 14336 for non-Error
free  = limit - (w - read_cached)                                     // u32 subtraction
if free < pad + need { … }
```

`ERROR_RESERVE = LANE_BYTES/8 = 2 KiB`. The Error reserve exists **precisely to be exercised** in the state where the used span exceeds `LANE_BYTES - ERROR_RESERVE` — v2's own test 5 constructs that state. There, a subsequent non-`Error` emit computes `14336 - 15000` in `u32` = `4_294_966_632`, the guard `free < pad + need` is **false**, and the producer writes at `off = w & MASK` over bytes the consumer has not staged. The consumer then walks a torn header and calls `decode` through a corrupted `*const LogSite`. That is the same UAF class B1 was folded to remove, re-entered through the arithmetic instead of through the ordering, on the designed-and-tested path.

**What (corrected).** Never block. Admission control is formulated so that **no unsigned subtraction can go negative**:

```
CAPACITY = LANE_BYTES - 1                    // one slot reserved: distinguishes full from empty
used     = w.wrapping_sub(read_cached)       // INVARIANT: used <= CAPACITY   (induction below)
avail    = CAPACITY - used                   // cannot underflow, by the invariant
budget   = if level == Error { avail } else { avail.saturating_sub(ERROR_RESERVE) }
if budget < pad + need { refresh read_cached (Acquire); recompute; if still short => drop }
```

**Why this form is sound and the old one was not.** The invariant `used ≤ CAPACITY` is *inductive over the producer's own admissions*: `read_cached ≤ read ≤ w` always (the consumer only advances `read` toward `w`, and `read_cached` is a stale copy of `read`), and the producer only ever publishes `w' = w + pad + need` after proving `pad + need ≤ avail = CAPACITY - used`, hence `used' ≤ CAPACITY`. The base case is `used = 0`. The reserve is now applied as a **`saturating_sub` from the available space**, not as a subtraction from the capacity, so the "reserve already eaten" state yields `budget = 0` — a refusal — instead of a 4-billion-byte licence. `saturating_sub` on `u32` lowers to `sub` + `cmov`: still branchless, one extra instruction on a path that already has a compare.

**Counters accumulate in `u64`; they never saturate** *(X4 REVERSED by S8)*. v3 made `dropped`/`dropped_bytes` `AtomicU32` with a saturation guard, on the argument that "an 8-byte RMW is more expensive". That rejection does not survive: on x86-64 `lock xadd` costs the same at 4 and 8 bytes, **and the lane-owned cell needs no RMW at all**. v4 adopts `boyko_diag::loss` (A3) wholesale:

- a per-lane `LossCell { count: u64, bytes: u64, _pad: [u8; 48] }` (64 B, cache-line-owned), written by the **lane owner** with plain `u64` load/store — single-writer, no lock prefix, the same argument this plan already makes for `SAMPLE_CTR`;
- the consumer folds into a `LossTotal { count: AtomicU64, bytes: AtomicU64 }` and clears with **`fetch_sub(observed)`, never `store(0)`** — a `store(0)` loses any increment landing between the consumer's load and its clear;
- the class is `boyko_diag::LossClass` (`Overflow` for an admission refusal, `Unclaimed` for E6, `Refused` for `TOO_LARGE`, `Sink` for an `ECS_HANDOFF` or callback loss, `Rotation` for E21).

Two things fall out. `SATURATED` and its census token are **struck** — a `u64` at 66 M offers·s⁻¹ needs ~8 800 years to wrap, so there is no ceiling state left for a reader to mistake for a number, and a token the census could never let a reader *compare* stops existing. And with the counters in `boyko_diag`, **the report of a loss is a read of a counter, not a record that can itself be dropped** — the second-order defect (the profiler's drop report being dropped and counted as a *logger* drop) is removed by construction rather than mitigated. Gate **G11**'s subject changes accordingly (§Gates).

**One aggregated drop report per drain, not one per lane** *(fixes F24)*. v2 emitted a synthetic `boyko-W0102` "per drain per lane": at 125 Hz × 128 lanes that is ~16 000 sink-generated records·s⁻¹ against a stated ~500 K·s⁻¹ formatting budget — 3 % of the budget spent by the drop reporter competing with the drops it reports, and unbudgeted. v4 emits **one** `W0102` per drain carrying `lanes_affected`, `records`, `bytes` and the `LossClass` breakdown: 125 records·s⁻¹, a fixed cost. Per-lane detail lives in the census, which is polled, not streamed.

**Lane-exhaustion fallback** (M26; **destination corrected by B9**): a thread with no lane does **not** silently drop `Warn`/`Error`. It falls back to the synchronous channel — `write_oracle_line`, bounded per §Decision 9c — for those two levels only, and counts `Info`/`Debug`/`Trace` as `LossClass::Unclaimed`. v3 left the promise inert in the shipped configurations, because `write_oracle_line` targeted stderr **unconditionally** and `shipping`/`shipping-min` configure no console sink. v4 fixes the destination, not the sentence: `write_oracle_line` fans out to **every configured synchronous destination** (§Decision 9c, "the durable fan-out"), which in `shipping`/`shipping-min` is the boot-opened crash file. The cost is paid only in the exhausted case, and a test harness that exhausts lanes therefore cannot lose a severe record **in any profile** — which is what v3 claimed and did not deliver.

**Why.** Blocking on `error!` inside a driver callback under a storm is a deadlock. Silent loss turns a logger into a source of false confidence — the exact class this campaign exists to kill.

**Alternatives rejected.** *Block-on-full* (spdlog's default) — a mutex by another name. *Overrun-oldest* — destroys the record that reported the cause in favour of the one that reported the consequence. **This rejection is what made v3's `shipping-min` structurally wrong** (B8): with no consumer, drop-newest fills the lanes with boot-time records within seconds and refuses everything up to the crash, so the profile whose only product is a crash log was guaranteed not to contain the crash. v4 does **not** answer that by switching to overrun-oldest — the argument above still holds — but by giving `shipping-min` a **real consumer** (Decision 10, Decision 25). Drop-newest with a consumer is a bounded loss; drop-newest without one is a guarantee of the wrong contents.

### Decision 6: One diagnostic-code registry, kept honest by eight mechanical checks over a SPECIFIED walker *(fixes F5, F20)*
**What.** `crates/boyko_log/src/codes.rs` holds a single `codes! { … }` invocation generating: a `pub const` per code, a **dense** `static DIAGNOSTICS: [DiagInfo; N]` sorted by number, a dense `code_idx` per code (the `RATE` index — M12), and `explain()`. A literal `"boyko-…"` outside the registry is a build failure. Class is a **type** property: `warn!` takes `WarnCode`, `error!` takes `ErrorCode`, `PanicCode` is distinct — a class mismatch does not compile.

**The checks** live in `crates/boyko_log/tests/code_registry.rs` — an **integration** test, because `cargo test --workspace --lib` does not build `tests/`, a blind spot that cost this repo four commits.

#### The walker: ONE pass, THREE streams, and every check names the streams it consumes *(fixes F5)*

v2 specified no walker, and its checks 3 and 6 then required **opposite** behaviour from the one it did not specify. Measured in this tree:

- `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences of `boyko-B1802`; **18** are inside `/// # Panics` doc comments (`:267`, `:283`, `:303`, `:333`, `:365`, …), **1** is the panic message string (`:867`), and **5** are `#[should_panic(expected = …)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). *(The review said 28 doc-comment sites; the measured number is 18. The finding stands — the count does not.)*
- `crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs:302,304,309,314,317,318` mentions `boyko-B9001` / `B9002` / `B9004` / `B9101` in doc comments, plus a `boyko-B900x` **pattern** at `:302` that is not a code at all.
- `crates/boyko_ecs/src/ecs/core/system/params/diagnostics.rs:46` — `/// Diagnostic code: \`boyko-B0002\`.`

So: a substring scan that includes comments makes check 3 (no orphans) satisfiable by writing a `# Panics` line — v1's vacuity relocated from `.md` into `.rs` — while a scan that excludes comments makes check 6 (panic-class placement) red on those same 18 sites the day L2 lands. Both cannot hold under one unspecified walker.

**The walker is specified.** One pass over each `.rs` file produces three disjoint streams, and it is the **same walker** that backs `print_census.rs`, so its `#[cfg(test)]`-region rule and its `src/bin/` exclusion are written once and exercised by two tests:

| Stream | Contents | Consumed by |
|---|---|---|
| **CODE** | source text with `//`/`///`/`//!` line comments, `/* */` block comments, string/char literals and `#[cfg(test)]` regions removed — **including whole files reached by a `#[cfg(test)]`-gated `mod` declaration, per the cross-file rule below (B7)** | checks 3, 3b, 6, 7 and the print census |
| **LIT** | the contents of string and char literals only | check 4 |
| **TEXT** | the whole unstripped file, plus the **explicit doc directory list** below — **`docs/archive/**` is NOT in it (B6)** | checks 0, 4 |

Check 3 matches a **standalone identifier token** (`B1802`, or the path form `codes::B1802`) in CODE — never a substring of `boyko-B1802`, which after stripping does not exist in CODE at all. Check 4 matches the `boyko-[BEW]\d{4}` **literal** in LIT ∪ TEXT. Check 6 runs over CODE, so doc mentions and the in-`src` `#[cfg(test)]` `should_panic` strings are invisible to it. The walker is deliberately not a Rust parser; its failure mode (a `#[cfg(test)]` block over-reaching to end-of-file) is the same one `scripts/check_hotpath_exceptions.py:164-201` already documents and accepts for the same reason.

#### The `#[cfg(test)]` rule is CROSS-FILE, because the corpus is *(fixes B7)*

v3's rule was within-file, and the ledger then claimed it excluded 23 in-`src` test prints. Measured against the tree, only **16 of the 23** are excludable that way:

| File | Prints | How it is gated | Within-file rule sees it? |
|---|---|---|---|
| `crates/boyko_rhi_vulkan/src/compute/tests.rs` | 16 | `#[cfg(test)]` **inside** the file (`:6`, `:138`, `:257`, `:872`, `:1610`, …) | **yes** |
| `crates/boyko_sdf_math/src/brick/tests.rs` | 3 | parent: `crates/boyko_sdf_math/src/brick.rs:1829-1830` — `#[cfg(test)]` / `mod tests;` | **no** |
| `crates/boyko_physics/src/solver/colored_tests.rs` | 4 | parent: `crates/boyko_physics/src/solver/colored.rs:3198-3200` — `#[cfg(test)]` / `#[path = "colored_tests.rs"]` / `mod tests;`. The file's only `cfg(test)` match, `:2569`, is inside a **comment** | **no** |

So 7 of the 23 would be classified as production sites and `print_census.rs` would fail on them — forcing test-only sites into `print_allowlist.txt`, which is exactly the allowlist-laundering this design says it prevents.

**The rule, stated so it can be implemented without a Rust parser.** In one pre-pass the walker collects, from every `.rs` file, every declaration matching `#[cfg(test)]` (or `#[cfg(any(test, …))]`) followed within the next two attribute/whitespace lines by an optional `#[path = "REL"]` and then `mod NAME;`. Each such declaration marks a **file** as test-only: `REL` when `#[path]` is present, otherwise `NAME.rs` or `NAME/mod.rs` resolved against the declaring file's directory. Marked files contribute **nothing** to CODE and nothing to the print census. `#[cfg(any(test, …))]` is treated as test-only for exclusion purposes and **is listed by name in the walker's report**, so a file excluded because of a `feature`-plus-`test` disjunction is visible rather than silently absent; the engine has no such site today and a new one must be seen. `#[cfg(all(test, …))]` is also test-only. `#[cfg(not(test))]` is not.

**Arithmetic re-derived against the rule that will actually run**: `179 − 58 (CLI bins) − 23 (test-only files: 16 within-file + 7 cross-file) = 98` occurrences to disposition — the same 98, but now reachable by the specified rule rather than by a rule that would have missed 7. After S1 removes the 20 measurement rows the walker's site denominator is **≤ 78** (§Integration ledger).

#### The TEXT corpus is an explicit directory list *(fixes B6)*

Measured this session (`grep -roE 'boyko-[BEW][0-9]{4}' docs --include='*.md'`): **75 occurrences across 13 files**, split `docs/LOGGING-SYSTEM-PLAN.md` **41**, `docs/archive/**` **29 across 10 files**, `docs/SYSTEMS.md` **3**, `docs/PROFILING-SYSTEM-PLAN.md` **2**. *(The round-3 review said 70; the orchestrator addendum said 75 with an archive share of 27. 75 is right; the archive share is **29**, re-measured here.)*

`docs/archive/` holds completed-phase planning documents that **must never be edited again**, and it contributes three codes that exist in **no source file and no current document**: `B9000`, `B9003` and — a case neither the review nor the addendum named — **`W9003`** (`docs/archive/PHASE-15-PLAN.md:471`). *(Written here **without** the `boyko-` prefix, deliberately: see the paragraph two below. This document is inside the corpus check 4 scans.)* Seeding those as `Pending(rung)` is wrong: `Pending` promises a future emitter, and these promise nothing. Two consequences:

1. **TEXT's corpus is an explicit directory list**, written in the test and not inferred: `docs/*.md` (top level), `docs/diagnostics/**.md`, `docs/ru/*.md`, `book/src/**.md`. **`docs/archive/**` is excluded**, with the reason in the test's own comment. Check 0's non-empty assertion and its pinned sentinel (`boyko-W1501`) still apply to the reduced corpus, so a mis-resolved root is still caught.
2. **A `Historical` row status exists** for the case where an archived path is ever re-included, or where a code appears in a frozen artifact this repository will not edit: zero emitters permitted, **no `docs/diagnostics/` page required**, and it never becomes `Live`. Check 3b (`Pending ⇒ 0 emitters`) applies to `Historical` too; check 3c (`Pending == 0`) does not, because a `Historical` row is not migration debt. No row is `Historical` at L2 — the three archive-only codes are simply outside the corpus — and the state is defined now so that re-including a directory is a one-line change and not a redesign.

**This document is itself in the corpus, and that has a consequence v3 missed.** v3's check-4 row spelled its illustrative red state as the **full prefixed literal** for code W9003 — a code that is real, unregistered, and present in `docs/archive/PHASE-15-PLAN.md:471`. Because check 4 matches the `boyko-`-anchored pattern over TEXT, and TEXT includes `docs/*.md`, v3's own planning document would have **red check 4 permanently, on itself, from the day the gate was armed**. The rule this document now obeys, and states so the next author does not undo it: **a planning document may name a code by its bare number-and-class (`W9003`, `E0115`) but may write the prefixed literal only for codes the registry actually carries.** The gate log demonstrates check 4's red state by writing the prefixed literal into a **scratch file inside the corpus**, then deleting it — never by carrying it in a committed document. Every prefixed literal remaining in this file names a code this plan registers.

#### Doc-page debt nobody counted *(B6, measured)*

`boyko-B9004` (5 occurrences) and `boyko-B9005` (7) exist in `crates/**/*.rs` and in **no document at all**. Check 2 requires `docs/diagnostics/<code>.md` with three named sections for every `Live` row, so these two owe pages that do not exist and that v3's L2 exit criterion did not count. They are named in L2's line items with their rung: both are `Live` from L2 (they have emitters today), so both pages land **at L2**, in the same commit as the registry rows.

#### `Live` vs `Pending`: how L2 commits alone on a grandfathered corpus *(fixes F20)*

L2 seeds the registry with the 9 existing codes, but the *identifiers* `codes::B9001`, `codes::W1501`, … do not appear in CODE until L6/L7/L8 migrate the emitters. A check-3 that scans identifiers therefore reds at L2, and one that scans literals is vacuous. Each registry row carries a status:

- **`Live`** — check 3 requires ≥1 CODE occurrence, **and check 2 requires its doc page**. Register a `Live` code and emit it nowhere ⇒ **red**.
- **`Pending(rung)`** — check **3b** requires **zero** CODE occurrences. Emit a `Pending` code ⇒ **red**, which forces the row to be flipped to `Live` in the same commit that lands the emitter. A `Pending` row cannot rot silently, because the day it acquires an emitter it reds. **Check 2 does NOT cover `Pending` rows** *(S6)*: a `Pending` row owes its `docs/diagnostics/<code>.md` in the same commit that flips it to `Live`. Requiring the page at seeding time would owe L2 seventeen pages for codes with no emitters — doc-rot manufactured by a gate.
- **`Historical`** — zero emitters permitted, **no doc page required**, never becomes `Live` (B6). For codes that exist only in frozen artifacts this repository will not edit. Check 3b applies; check 3c does not.
- Check **3c**, armed at L8c only: `Pending` count == 0. This is the migration's real exit criterion, and it is one integer. `Historical` rows are excluded from it by definition.

#### The eight checks

Numbered **0 through 7**. Checks `3b` and `3c` are legs of check 3, not additional checks — the count is eight, and `codes_tidy!` generates all eight for a downstream table.

| # | Check | Stream / corpus | Red state that must be demonstrated once |
|---|---|---|---|
| 0 | **Corpus is non-empty**: `files_scanned ≥ 500`, and the pinned sentinel `boyko-W1501` is found | TEXT | point the walker at a wrong root → red |
| 1 | Numbers strictly increasing ⇒ no duplicates (also a `const _: () = assert!`). **This is why two rows may never share a number even across classes** — `W0110` and a hypothetical `E0110` would break it, and `DIAGNOSTICS` is dense with `index == code_idx` | registry | add a duplicate |
| 2 | `docs/diagnostics/<code>.md` exists, non-empty, has `## What happened` / `## Why` / `## How to fix`. **`Live` rows only** *(S6)*; `Pending` and `Historical` are exempt | `docs/diagnostics/` | delete a section heading; **or flip a row to `Live` without its page** |
| 3 | **No orphans**: every `Live` code's identifier appears ≥1× as a standalone token | CODE, excluding `codes.rs` | register a `Live` code, emit it nowhere |
| 3b | **No premature emitters**: every `Pending` **or `Historical`** code's identifier appears **0×** | CODE | emit a `Pending` code without flipping its row |
| 3c | **Migration complete** (armed at L8c): `Pending` count == 0. `Historical` excluded | registry | leave one row `Pending` |
| 4 | **No undeclared**: every `boyko-[BEW]\d{4}` literal resolves to a registry entry | LIT ∪ TEXT (the explicit directory list; **`docs/archive/**` excluded**) | write the literal form of `W9003` into a scratch file inside the corpus. *(The full literal is deliberately not written in this committed document: it is a real archive code with no registry row, so carrying it here would red check 4 permanently — the self-referential failure v3 shipped.)* |
| 5 | Every `Live` `W`/`E` code is observed by ≥1 test, with `tests/untested_codes.txt` (a **data file**, excluded from its own scan) checked **in both directions** | `crates/**/tests/**`, `#[should_panic(expected=` | allowlist a code that has a test |
| 6 | Panic-class `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!` | CODE | emit a `B` code from a `warn!` |
| 7 | Every `LogTarget` impl in the workspace resolves to a `targets!` row or a `define_target!` expansion | CODE | hand-write a `LogTarget` impl |

**Why the corpus rules changed.** v1's check #3 was vacuous because check #2 *mandates* a doc file naming the code and v1's scan included `.md`. v2 narrowed the corpus to `.rs` and reintroduced the same vacuity through comments. v1's check #5 was self-defeating: the allowlist named identifiers and lived inside the file being scanned. Check #0 closes the third failure in the same family — a walker that resolves its root badly scans zero files and reports zero orphans, green. rustc's tidy pins a sentinel for exactly this reason.

**What these checks CANNOT claim.** They are engine-scope. They prove nothing about a game's or a mod's registry — which is why `codes_tidy!` (Decision 19) generates the same eight checks over a caller-supplied root and prefix, and why G9's assertion message says so in the failure text rather than in this document.

**Prior art.** None of Bevy / flecs / UE / Unity / spdlog / Quill / NanoLog ships a code registry. The prior art is compilers: rustc's numbered codes with a mandatory long-form `.md` and its eight tidy checks; Clang's named groups; MSVC's opaque numbers with per-code pages. rustc's experience is that the number is worthless without the mandatory explanation *and* the orphan check.

**Block map** — defined *around* **measured** existing occupancy, because codes are never renumbered. The "occupied today" column is not an assumption: it is `grep -roE 'boyko-[BEW][0-9]{4}' crates --include='*.rs'`, run this session, which returns **89 occurrences and exactly 9 distinct codes** — `B0002` (24), `B1802` (24), `B9001` (11), `B9101` (7), `B9005` (7), `B9004` (5), `B9002` (5), `B1801` (4), `W1501` (2). That set **is** the "9 grandfathered codes" L2 seeds, and the figure is confirmed rather than inherited.

| Block | Domain | Occupied today |
|---|---|---|
| `00xx` | ECS core / system params | `B0002` |
| `01xx` | `boyko_log` itself | new |
| `02xx` | `boyko_threadpool` | new |
| `03xx` | memory (`VmReservation`, `ComponentPool`) | new |
| `04xx`–`09xx` | components, query, change detection, events, assets, serialize | new |
| `11xx`–`14xx` | input, scene, physics, math/sdf | new |
| `15xx` | schedule sets & ordering | `W1501` |
| `18xx` | app / plugins | `B1801`, `B1802` |
| `20xx`–`27xx` | RHI, RHI-Vulkan, render, shaderdsl, UI, fontbake, image, GPU columns | new |
| `30xx` | host / runner | new |
| `90xx` | schedule **build** (historical) | `B9001`, `B9002`, `B9004`, `B9005`; **`B9003` permanent gap**; `B9000` and `W9003` appear only in `docs/archive/**`, which is outside the corpus (B6) |
| `91xx` | world binding | `B9101` |
| **`92xx`** | **profiling** — reserved at **L2** *(S6)* | none. **Measured**: the `9xxx` band is already occupied by `B9001`/`B9002`/`B9004`/`B9005`/`B9101`, but `92xx` itself is free. The profiling plan asserted availability without checking; this row records the check |

The `15xx`/`90xx` split is a historical artifact, documented as such, and **must not be tidied**: renumbering would break the book, the `#[should_panic]` assertions and the never-reuse rule simultaneously.

**The `92xx` reservation, and why it lands at L2 and not later** *(S6)*. `docs/PROFILING-SYSTEM-PLAN.md` already contains two code literals — `boyko-W9207` (`:200`) and `boyko-E9204` (`:376`), both measured this session — and check 4 scans `docs/*.md`. So the day L2 arms its checks, an already-committed document reds check 4 unless the rows exist. L2 therefore seeds **all 17 `92xx` rows as `Pending(<profiling rung>)`**, owning no doc pages (check 2 is `Live`-only) and no emitters (check 3b). Each profiling rung that introduces a code carries three explicit line items — registry row flip, doc page, and one observing test (check 5) — and those line items belong to the profiling plan's rung table, not to this ladder. The `W92xx` conditions themselves are raised inside `boyko_diag`/`profiling_abi` as sticky `DiagFlag`s and **emitted from `boyko_ecs`'s profiling fold**, because the substrate is diagnostically mute; this crate is the emitter of record for none of them.

### Decision 7: `Info`/`Debug`/`Trace` carry NO code; `Warn`/`Error` MUST
Different macro arities enforce it. A code is a promise of documentation, stability and an explanation; extending it to trace chatter makes the registry meaningless and check 2 unenforceable, and making it optional reproduces today's state — nine codes across thousands of diagnostics.

### Decision 8: Rate policy is DECLARED on the code and APPLIED per site; `Once` is a site-local latch that degrades to a pure load *(fixes M11, M12, F11; extended by X3)*

**What.** Each `W`/`E` code declares `Every` / `Once` / `OnceCounted` / `EveryN(n)` / `MinIntervalMs(ms)` in the registry. The *mechanism* differs by policy, and that distinction is the fix for F11:

| Policy | State lives in | Scope | Steady-state cost |
|---|---|---|---|
| `Once` | a macro-generated `static FIRED: AtomicBool` **beside the call site's `LogSite`** | **per SITE** | one `Relaxed` load from a site-private line, not-taken branch |
| `OnceCounted` | the same site-local static + an `AtomicU32` | per SITE | one load; **one RMW per suppressed occurrence** — opt-in, cost stated at the declaration |
| `EveryN(n)`, `MinIntervalMs` | `static RATE: [RateSlot; MAX_CODES]`, dense `code_idx` | per CODE | one RMW per occurrence |
| `Every` | — | — | nothing |

**Why `Once` had to become per-site** *(F11)*. `RATE` is indexed by `code_idx`, so a code-scoped `Once` fires **once per code, not once per site**. Read against the tree: the migration routes `crates/boyko_rhi_vulkan/src/device.rs:3100`, `:3158` and `:3189` — three independent capability degradations — through **one** code `W2102` with `RatePolicy::Once`. A device lacking all three would report **one** and silently lose two, uncounted (and `Once` deliberately does not count). That defeats the migration's own stated purpose, which is `crates/boyko_app/src/host.rs:228-233`'s written argument that a RELEASE-build degrade-to-off must be observable. `W2202` has the same shape across `bindless.rs` and `mesh_geometry_table.rs`.

The resolution is not to mint three codes — a code names a *class of condition*, and three near-identical codes make `explain()` worthless — but to put the latch where the diagnostic value is: **the site**. The macro expands, next to the `&'static LogSite` it already emits, a `static FIRED: AtomicBool = AtomicBool::new(false)`. Cost: 1 byte of `.bss` per `Once` site. Benefit beyond correctness: the steady-state load hits a **site-private, never-contended** line instead of a shared `RATE` line, so this is strictly cheaper than v2 on both axes.

```
Once:  if FIRED.load(Relaxed) { return; }          // steady state: load only, private line
       if !FIRED.swap(true, Relaxed) { emit }      // exactly one RMW, ever, per site
```

**Why the no-store property matters.** The audit found five hand-rolled latches with two implementations, one wrong: `crates/boyko_render/src/render_path_config.rs:311-313` and `:335-337` execute `swap(true, Relaxed)` on a shared line **inside an `#[inline]` per-frame reader, every frame forever** once the divergence holds. v1's replacement kept a per-frame `suppressed.fetch_add` — the same defect wearing a policy name.

**Suppression is reported as policy, and the absence of a count is itself printed** *(fixes F10)*. The review is right that v2 contradicted its own Goal: "loss is counted and reported; never silent" against "the `Once` count … is **not reported**", with the census reporting neither (`suppressed` is neither `records` nor `dropped`). v3 resolves it in three moves rather than by softening a sentence:

1. The Goal now says suppression is **not loss** and is reported **as policy** (§Goal, functional bullet 3).
2. **The census prints one row per FIRED SITE, from a real observation** *(v3's per-code line is deleted by M1)*. v3 printed `rate=Once fired=1` per code. With the latch per-site and `RATE` unused by `Once`, **nothing aggregated per code** — no structure enumerated the `FIRED` statics and `TARGET_STATS` is per-target — so `fired=1` was a literal, and it was wrong in exactly the three-site `W2102` case F11 was raised for. v4 makes the enumeration exist:

   ```rust
   /// Intrusive, insert-only, allocation-free. The NODE is the per-site static
   /// the macro already expands; the list only adds a `next` pointer to it.
   #[repr(C)] pub(crate) struct OnceSite {
       site: &'static LogSite,
       suppressed: AtomicU32,      // OnceCounted only; 0 and unread for plain Once
       next: AtomicPtr<OnceSite>,  // CAS-pushed exactly once, on this site's single fire
   }
   static ONCE_SITES: AtomicPtr<OnceSite> = AtomicPtr::new(core::ptr::null_mut());
   ```

   The push is a `#[cold]` CAS loop executed **once per site per process**, on the same branch that already performs the single `FIRED.swap(true)` — so the steady-state path is still a pure `Relaxed` load from a site-private line and **nothing is added to the budgeted path**. The census walks the list (`Acquire` on `next`) and prints, per fired site:

   ```
   LOG-ONCE code=W2102 site=device.rs:3100 fired=1 suppressed=UNCOUNTED(by policy)
   LOG-ONCE code=W2102 site=device.rs:3158 fired=1 suppressed=UNCOUNTED(by policy)
   ```

   A site that never fired is simply **absent from the list**, and its absence is the datum. `OnceCounted` rows carry a real integer in `suppressed=`. `RateSlot::fired` is **deleted** — it was dead the moment `Once` stopped using `RATE` (M1).
3. A code whose suppressed count genuinely matters declares **`OnceCounted`** and pays one RMW per suppressed occurrence — at its own declaration site, visible in the registry, with the cost written in the row. The engine's own `W2102`/`W2202` use plain `Once`; a game is free to choose otherwise.

**`EveryN(n)` requires `n` to be a power of two** *(X3)*, enforced by `const _: () = assert!(n.is_power_of_two())` inside `codes!`, so the test is `count & (n-1)` instead of `count % n`. v2's arbitrary `n` mis-samples across the `u32` counter wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper: an `and` for a division. Strictly better on both axes.

**Layout.** `RateSlot` is 64 B, one per cache line — four unrelated codes sharing a line (v1's 16 B slot) is false sharing between subsystems that have nothing to do with each other. `MAX_CODES = 512` ⇒ 32 KiB, in the same `.bss` regime as `LANES`.

### Decision 9: `report!` is DELETED. No engine code writes stdout at all *(re-cut by S1)*

v3 specified `report!` as a synchronous stdout macro carrying `VB-P1d` / `VB-P4` / `VB-SV0-S1.5` and the R0 table. **S1 gives the measurement channel to the profiler end to end**, so `report!` is deleted from this plan together with mandatory test 16 and L8b's 20 measurement rows.

**The rule that replaces it, stated once** *(S7)*:

> **stdout is written by exactly one thing in this workspace: `boyko_shaderdsl`'s CLI bins.** Nothing in the engine, the logger or the profiler writes stdout, ever. The measurement channel's durable output is the profiler's **artifact + binary telemetry stream (files)**, rendered by `tools/prof_decode` offline and by the `boyko_ui` overlay in-process.

**Why `report!` could not survive, with the measurement.** v3 justified it by "the lines are a machine API". They are — but the review that examined the seam found **six** consumer files, not one: `crates/boyko_app/tests/vg_occ_split_timing.rs`, `vb_bench_totality_gate.rs`, `vb_bench_query_validation.rs` (which uses the line as a *liveness witness* that the reset and every timestamp write executed), `vg_decidability_floor.rs`, `vb_p1d_cull_shade_bench.rs` and `sv0_deferred_term_bench.rs` (which transcribes printed lines into test source). All six exist; all six were checked. `vg_decidability_floor.rs` is decisive: the profiler's own `band = max(floor, twin)` consumes a **floor produced from that stdout line**, so keeping the line as `report!` would freeze a text stdout contract for measurement permanently *and* hand every machine-parsed line the `OUT_LOCK` steal interleave (G18 concedes it) — the interim design this project has standing instructions never to propose.

**The cost to this plan, stated plainly.** L8b's headline "20 sites → `report!`, text unchanged" drops to **zero rows**; the migration denominator falls from ≤ 98 to **≤ 78**; test 16 and the `report!` half of `print_allowlist.txt` are struck; open question 1 (a schema-versioned TOML form for `report!`) is moot and struck. The cost to the *engine* is larger and belongs to the profiling plan: six consumer files are rewritten in one commit and **every published floor number is invalidated**, because a floor measured on a different instrument bounds nothing about this one.

**What survives.** `OUT_LOCK` is **not** deleted — its remaining callers are enumerated in Decision 9c, and Decision 9c is retained unchanged. `write_oracle_line` is not deleted; it gains the durable fan-out (B9). The byte-frozen `[vk-validation]` contract is untouched (Decision 9b). **This plan moves no stdout contract**, which was true in v3 and is true in v4 for a stronger reason: it no longer writes stdout at all.

**RED for the deletion** *(S1)*: after profiling rung 7, `rg 'VB-P1d |VB-P4 pass=|VB-SV0-S1\.5 ' crates/*/src` must return **zero**. Leaving one `println!("VB-P4 pass=…")` in `runner.rs` is caught twice — by that grep and by L8c's `print_census.rs`.

### Decision 9b: The validation messenger is NOT TOUCHED AT ALL — v1's migration stays withdrawn, and v2's "harmless" edit is withdrawn too *(fixes B8 and F12)*
**What.** `crates/boyko_rhi_vulkan/src/debug.rs`'s callback keeps its `eprintln!("[vk-validation] {}", msg.to_string_lossy())` at `:114`, byte for byte, on `stderr()`'s own lock. Nothing about it changes. It is added to `tests/print_allowlist.txt` with the reason "byte-frozen gate-oracle channel; see Decision 9b" — and because that allowlist is checked **in both directions**, a future removal of the site reds the tidy test rather than silently orphaning the entry.

**Why v2's edit is withdrawn** *(F12)*. v2 justified touching the site by "removal of the per-message `to_string_lossy()` allocation". That justification is largely false and the edit is actively harmful:
- `CStr::to_string_lossy()` returns `Cow::Borrowed` for valid UTF-8 — **no allocation on the normal path**. It allocates only for invalid UTF-8, which is not the path any gate runs.
- Writing "the `CStr` bytes directly" **changes the emitted bytes** exactly in the invalid-UTF-8 case (today `U+FFFD`, after: raw bytes) — on a channel this document declares byte-frozen and gate-oracle, pinned byte-exact including the trailing space at `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118`.
- `eprintln!` currently takes `stderr()`'s own lock. Moving the site to `OUT_LOCK` would let the ~90 surviving `eprintln!` sites interleave *inside* a `[vk-validation]` line — a regression **introduced by M24's own fold**.

Trading a non-allocation for a byte change and an interleaving hazard, on the one channel whose value is that it has not moved, is a bad trade. The site stays.

**Why the channel stays synchronous.** Verified this session at `scripts/golden.ps1:201,226,232`: the scan runs over the child's *merged* stdout+stderr file and prints `VALIDATION: clean (0 messages)` in green at zero. Today the message is on the wire before `vkQueueSubmit` returns. Behind a 16 KiB lane drained ≤ 8 ms later, three loss modes are all reachable *in exactly the runs the gate exists for*: a storm overflows the lane (a storm is what an error looks like); an error preceding a driver abort loses everything undrained; a rate policy suppresses. Each yields green. **Decision 9's own rule — a gate whose evidence can vanish is worse than no gate — applies here verbatim, and v1 violated it.**

**The E12 conflict is resolved, not finessed.** "No lock, no syscall" is a rule about **frame-hot paths**. A validation callback under an enabled validation layer is not one: validation is off by default, and when on, the run is already an order of magnitude slower. With the site untouched, the conflict does not even arise — the lock in question is `stderr()`'s, which predates this plan.

**The shared-handle clause, which is what makes S7's stderr rule buildable.** Both stderr producers — this messenger and `boyko_log::write_oracle_line` — write through `std::io::stderr()`'s **own handle**, never a raw fd, never `libc::write`. That is what makes them share stderr's inner lock, so **neither can splice a line into the other**, and `golden.ps1:226`'s line-start match on `[vk-validation] ` keeps holding under concurrency. *Ordering* between the two is undefined and stated as such: a log line may land between two `[vk-validation]` lines. **Line integrity, not ordering, is what the gate consumes** — and line integrity is exactly what the shared handle buys. Under an `OUT_LOCK` **steal** two `write_oracle_line` outputs may interleave with each other; `OUT_STEALS > 0` in a golden run remains a defect signal (Decision 9c, E25).

**What is *added*:** `boyko-E2101` (below) and the `LOG-CENSUS`, both of which make *absence* loud rather than making presence prettier. Neither writes to the messenger's channel. **Nothing is added that names a `vk-validation` *target***: see M4 in §sync-validation confrontation — a census row for a target no record can ever reach is a green-because-it-cannot-fail row wearing the vocabulary invented to prevent them, and it is deleted.

### Decision 9c: `OUT_LOCK` — bounded acquire, re-entrancy-aware, unwind-safe, and it steals rather than hangs *(fixes F8)*

**`OUT_LOCK` survives S1.** Deleting `report!` deletes one caller, not the lock: the **complete remaining caller list** is `write_oracle_line` (which is itself the console sink, the sync-routed targets of Decision 20, the lane-exhaustion fallback of Decision 5, the pre-boot / post-`shutdown()` `Warn`/`Error` fallback of Decision 12, the panic message, and `flush()`'s timeout line) plus `SINK_REQ` writes. Five of those seven are error-of-the-error paths, so the protocol below is if anything **more** load-bearing after S1, and G18 keeps its subject unchanged.

v2 specified an `AtomicBool` spin lock with **no bound, no release-on-unwind and no re-entrancy story**. Three concrete hangs followed, each on the error-of-the-error path — the one place a logger must not fail:

- **E14** ("panic inside a sink — the sink catches, **direct-writes**, continues"): if the panic happened while the sink held `OUT_LOCK`, the direct-write is a non-reentrant self-deadlock.
- **`flush()`'s timeout** writes `boyko-E0105` via `write_oracle_line` (§Algorithms D step 5) — a *bounded* wait terminating in an *unbounded* one.
- A `Display` panicking inside a formatting call leaked the lock permanently; the panic hook's flush then hung the process.

Against an invariant the same document states as "no new hang class", citing `vb_bench_totality_gate.rs:48-49`. The protocol is therefore specified, not assumed:

```rust
static OUT_OWNER: AtomicU64 = AtomicU64::new(0);   // 0 = free; else an opaque thread token
static OUT_STEALS: AtomicU32 = AtomicU32::new(0);
static OUT_REENTRANT: AtomicU32 = AtomicU32::new(0);

/// RAII. `Drop` releases on the normal path AND on unwind.
struct OutGuard { mode: OutMode }   // Held | Reentrant | Stolen
```

1. **Format before you lock.** Every caller renders into a caller-owned stack buffer first. No user `Display` and no `core::fmt` runs inside the critical section, so an unwind cannot originate there.
2. **Re-entrancy is detected, not deadlocked.** Acquire is `CAS(0 → my_token)`. On failure, if `OUT_OWNER == my_token` the caller is re-entrant (the E14 case): the guard is `Reentrant`, the bytes are written **prefixed by a newline** so they cannot corrupt the *start* of the outer line, and `OUT_REENTRANT` increments. The census reports it.
3. **Acquire is bounded.** Spin with `spin_loop()` backoff, then `yield_now()`, to a **50 ms** deadline. On expiry the writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not. This is the explicit trade, and it is the only shape compatible with Invariant 6.
4. **Release is unwind-safe** by construction — `Drop` on `OutGuard`, and the guard is the only way to obtain write access.
5. **The panic hook and `flush()`'s timeout path use the same bounded acquire**, so no bounded wait terminates in an unbounded one.

#### The durable fan-out *(fixes B9)*

v3's `write_oracle_line` targeted **stderr unconditionally**, and three separate mechanisms leaned on it for durability: the sync route ("this must be on disk before the next instruction", Decision 20), the lane-exhaustion fallback for `Warn`/`Error` (Decision 5, E6), and E22's crash mitigation. In `shipping` and `shipping-min` there is no console sink, so all three wrote to a stream nothing collects — inert in exactly the configurations they exist for.

**What is true about stderr on this tree, measured**: `grep -rn windows_subsystem crates src` returns **nothing**, so today every binary is a console-subsystem binary and stderr is a valid handle. The invalid-handle case is a **future** shipping configuration (`#![windows_subsystem = "windows"]`), not a current fact, and this document does not claim otherwise. The *durability* defect, however, is present today in every profile including `dev`: stderr is neither the log file nor `fsync`ed, so "durable-on-write" was already false.

`write_oracle_line(prefix, body)` therefore writes to **every configured synchronous destination**, under one `OutGuard`:

| Destination | Present when | Cost |
|---|---|---|
| `std::io::stderr()`'s handle | a `ConsoleSink` is configured (`dev`, `editor`) | one `write_all`, sharing stderr's inner lock (S7) |
| the **crash sink's** file handle, opened at `boot()` | a `CrashSink` is configured (`shipping`, `shipping-min`, and `dev` on request) | one `write_all` on an append handle; `sync_data()` **only** when `LogConfig.sync_durable` is set |
| the file sink's handle | **never** — that handle is owned by the sink thread and is not reachable synchronously without adding a second consumer | — |

Three consequences, stated rather than implied. **(i)** "Durable-on-write" now means what a `write_all` means: the bytes have left the process. Reaching the platter additionally needs `sync_data()`, which costs ~0.1-10 ms and is therefore **opt-in** (`sync_durable`, default off) — a per-target sync bit that also `fsync`ed would serialise the frame on the disk rather than on the format. **(ii)** The **~200 ns in Decision 20 is the uncontended, console-only cost**; with a crash-file destination it is one further `write_all`, and **contended it is bounded only by the 50 ms acquire deadline**, after which the writer steals (E25) — so a sync-routed record can interleave with another synchronous line, which is exactly the property such records exist to avoid. That is the trade, and Decision 20's "cannot claim" now names it. **(iii)** In `off` there is no synchronous destination, `write_oracle_line` is a no-op, and the mechanisms depending on it are dead — correct, because `off` deletes every call site.

**Gate G18** (L3-gate), now three-sided: (a) a thread that acquires `OUT_LOCK` and then panics releases it — a second thread's `write_oracle_line` completes within the deadline; (b) a re-entrant `write_oracle_line` from inside a sink panic handler **completes** and increments `OUT_REENTRANT` instead of deadlocking; **(c) fan-out**: with a console sink **absent** and a crash sink configured, a `Warn` from a laneless thread appears in the **crash file**. **Red states**: replace the guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it; restore the unconditional-stderr form of `write_oracle_line` ⇒ (c)'s crash file is empty ⇒ red. **What G18 cannot claim**: that the output is never interleaved. Under a steal it *is*, deliberately; `OUT_STEALS > 0` in the census is the honest report of that, and a nonzero value in a golden run is itself a defect signal.

**`OUT_LOCK` gets no row in `docs/HOT-PATH-EXCEPTIONS.md`** — see Invariant 1 for why that would red CI. Its justification lives in `sync_out.rs`'s module doc and here.

### Decision 10: One dedicated sink thread; adaptive park; **three** consumer disciplines, not two *(re-cut by B8)*
**What.** Default `SinkMode::Thread`: one thread, started at `boot()`, draining all lanes, staging, sorting by `tsc`, formatting once, fanning out. Park policy is **adaptive**: `park_timeout(0)` — immediate re-drain — while any lane yielded records last pass; `park_timeout(8 ms)` when every lane was empty. Producers **never** unpark (that is a syscall per record); `flush()` and `shutdown()` do.

**`SinkMode` has three variants, because two were not enough** *(B8)*:

| Mode | Consumer | Used by |
|---|---|---|
| `Thread` (default) | the resident sink thread | `dev`, `editor`, `shipping` |
| `Manual` | an explicit `drain()` call, single-caller | hermetic tests, CLI tools, the zero-alloc gate — **and nothing else** |
| **`Scheduled`** *(new)* | **`log_drain_system` in `Last`**, which takes `DRAIN_OWNER` and runs Algorithms C itself, once per frame, on the frame thread | **`shipping-min`** |

**Why `Scheduled` had to exist.** v3 gave `shipping-min` `SinkMode::Manual` and a crash sink. Nothing then drains: `flush()` returns `NoConsumer` immediately, admission control **drops new records** rather than overrunning oldest, and within seconds the 32 × 16 KiB of lanes hold nothing but boot-time records while everything up to the crash is refused. The profile whose only product is a crash log was structurally guaranteed **not to contain the crash** — and Decision 25 asserted `Manual` for it while this decision said `Manual` exists "for hermetic tests, CLI tools and the zero-alloc gate, and for nothing else", so the two contradicted each other on the page.

**Why draining from the schedule is admissible now, when v3 rejected it.** The rejection below stands on two grounds, and one of them has been removed:
- *"It makes the consumer MPSC when combined with any other driver."* **Answered by `DRAIN_OWNER`** (Decision 24, B5): the consumer role is a CAS-claimed token, so a schedule drain, a manual `drain()` and the crash drain are mutually exclusive by construction rather than by convention.
- *"It ties log liveness to a running schedule, so boot and shutdown diagnostics vanish."* **Not answered — and therefore stated as the profile's cost.** In `shipping-min`, records emitted before the first frame or after the last are covered only by the synchronous channel (`sink_can_accept()`'s pre-boot / post-shutdown fallback, Decision 12) and by the crash drain. That is a real hole in a real profile, written here rather than discovered by a support agent.

**The other cost, stated in the units it is paid in.** The drain runs **on the frame thread**, bounded per pass by `STAGE_BYTES` (256 KiB of staged records) plus the format cost of what it staged. At `shipping-min`'s `Warn` ceiling the expected volume is a handful of records per second, so the expected per-frame cost is ~0; the **worst** case is one `STAGE_BYTES` pass, which is why the pass is bounded at all. `Thread` mode remains the default everywhere it is affordable.

**The retained window, in RECORDS** *(B8 asks for this explicitly, because bytes are not what a reader wants to know)*: 32 lanes × 16 KiB ÷ ~40 B ≈ **13 100 records** across all lanes, ≈ **410 per lane**. Under `Scheduled` that ceiling is never approached — the window is "records since the previous frame" — and the crash drain therefore emits the records adjacent to the crash rather than the session's first 13 100. Under `Manual` with no caller (the v3 shape) the same 13 100 were guaranteed to be the *oldest* ones.

**Throughput, stated rather than deferred** (M19). At the default geometry, a lane holds `16 KiB / ~40 B` ≈ 400 records; with immediate re-drain under load, per-lane sustained capacity is bounded by the sink's formatting rate, not by the park interval. Design number: **≥ 500 K records·s⁻¹ aggregate** with a `core::fmt` cost of ~1-2 µs per formatted record on one thread. Consequence, stated plainly: **a `trace!` inside a per-entity loop is lossy by construction** — at 15 ns/record a single producer can offer 66 M records·s⁻¹ against a consumer two orders of magnitude slower. Gate **`sink_sustained_rate`** at L3 measures the knee and must show a nonzero drop count above it; the plan is not allowed to ship with the knee unmeasured.

**Alternatives rejected.** *Drain from an ECS system at `Last` **as the default*** — ties log liveness to a running schedule, so boot and shutdown diagnostics vanish; admitted for `shipping-min` only, with that cost written down (`Scheduled`, above). *Drain from the frame loop* — a syscall in the frame. *Making `shipping-min` overrun-oldest instead* — see Decision 5: it destroys the record that reported the cause in favour of the one that reported the consequence, and the profile does not need it once it has a consumer.

### Decision 11: The clock is `boyko_diag::clock` — this crate owns no clock at all *(re-cut by S4)*

**What.** `crates/boyko_log/src/tsc.rs` is **deleted**. Timestamps come from `boyko_diag::clock` (A1): `ticks()` (`rdtsc`, QPC fallback), `ticks_per_ns()`, `clock_epoch()`, `calibrate()`, `note_forward_jump()`, `invariant_tsc()`. Records store **raw ticks**; the sink reads the scale and the epoch from A1. Code **W0101** (invariant TSC absent) is **struck** — it had no reachable red state on any targeted machine by this plan's own N30, and the single invariant-TSC code is now the profiling plan's `boyko-W9207`. *(Named bare, per the corpus rule in Decision 6: a struck code has no registry row, so writing its prefixed literal in a scanned document would red check 4.)*

**Why one owner, and what the sharing actually buys.** Not speed: the boot saving is ~one `cpuid`, not 20 ms. **The benefit is agreement**, and this document says so rather than claiming a speedup. Without one owner a suspend/resume produces a profiler window quarantined as an epoch break and, in the same seconds, log lines whose printed wall times are wrong by the suspend duration **with no marker** — two artifacts that disagree, neither of which says why.

**`RecordHeader` carries the epoch, and the header does not grow** *(S4 left the choice to L3; here it is)*. v3's header was 20 B packed with `flags: u8` (3 bits used) and `_pad: u8`. v4 spends the pad: **`clock_epoch_lo: u8`**, the low 8 bits of `boyko_diag::clock_epoch()`, read as a register-resident global — no probe, no syscall, so the ≤ 15 ns row is unaffected and the `HEADER_BYTES == 20` const assert stands. Eight bits suffice because the sink is at most one park interval behind the producer, so at most one epoch boundary can lie between them: the sink reconstructs the full `u32` by comparing `clock_epoch_lo` against the current `clock_epoch()`. The sink **renders the epoch beside every timestamp**, so a record straddling a discontinuity is legible instead of merely wrong.

**The citation, corrected in v3 and carried.** The tree's note on the QPC-backed `Instant::now()` is at `crates/bench_bevy_vs_boyko/benches/profile_spawn.rs:229-231`: "each **pair** of `now()` calls costs **~20-30 ns**". v2 claimed "~25 ns/call and ~60 ns/pair — *measured*", which is 2× the cited source and attributes a measurement to a prose comment. The tree records ~20-30 ns per *pair*, i.e. ≥ 10 ns per call, and that number is a comment, not a recorded run. The argument survives with room to spare — a ≥ 10 ns clock inside a 15 ns whole-record budget is not a design. Tracy uses `rdtsc` guarded by an invariant-TSC check; AVX2 baseline ⇒ Haswell+ ⇒ invariant TSC since Nehalem.

**Cross-domain correlation, restated with what sharing gained.** A host clock is still not comparable to a Vulkan GPU timestamp without `VK_EXT_calibrated_timestamps` (per Khronos, device timestamps "cannot be compared even across separate submits within the same run"), and GPU correlation remains out of scope. What **is** now exact is **CPU ↔ log-record correlation**: same counter, same scale, same epoch, so a profiler sample and a log record can be placed on one axis without a fitted offset. That is the only cross-domain correlation v1 of either subsystem offers, and it exists *because* the clock is shared.

### Decision 12: No `LogHandle`; explicit `boot`/`shutdown`; `flush` never waits on a consumer that cannot answer *(fixes M16, M17)*
**What.**
- `boot(cfg) -> Result<(), LogBootError>`; state lives in process-lifetime statics. **There is no handle**, because v1's handle was `!Send + !Sync` (so it could not be a `Resource`, which requires `Send + Sync`) and its `Drop` would have shut the logger down at the end of `Plugin::build`.
- `shutdown()` is explicit, idempotent, called by `App` teardown and by the process-exit path.
- `flush() -> FlushResult` reads `SINK_STATE` **first**: `NotBooted` / `Manual` / `Exited` ⇒ return `FlushResult::NoConsumer` immediately. Only in `Running` does it bump `FLUSH_REQ`, unpark, and spin to a 2 s deadline, after which it direct-writes `boyko-E0105` and returns `FlushResult::TimedOut`.
- `shutdown()` sets an exit flag, unparks, then spins on `SINK_EXITED` to a 2 s deadline. **There is no `join`-with-timeout, because `std::thread::JoinHandle::join` does not have one** — v1 asserted a facility std does not provide, in the one place it promised "no new hang class". On timeout the thread is **detached** and `boyko-E0108` is written synchronously.

**Why it matters.** This repo has dozens of `#[should_panic(expected = "boyko-B…")]` tests and a panic hook that flushes. With v1's unconditional 2 s deadline, every such test in an unbooted binary would have paid the full timeout — a self-inflicted 60-second test suite.

#### The lifecycle is `boyko_app`'s, and it is stated once *(S5)*

Nobody but `boyko_app` may call `boot`/`shutdown`. The order is fixed:

```
BOOT     boyko_log::boot(cfg)  ->  App::new  ->  LogPlugin::build (binds LogRing/LogCensus)
         ->  ProfilerPlugin::build  ->  Profiler::arm()   [registers flush_on_panic in PRE_FLUSH]
FRAME    unchanged
TEARDOWN flush_gpu()  ->  Profiler::disarm()  ->  boyko_log::flush()  ->  boyko_log::shutdown()
```

**`flush_gpu` moves ahead of `flush`.** That single reordering is the whole fix for the teardown hole where GPU-side diagnostics were emitted after the logger had stopped accepting them.

#### `PRE_FLUSH` — the callback seam, owned here

```rust
/// .bss, claimed by CAS, holding `extern "C" fn()`. Called by flush(), by
/// shutdown(), and by the panic hook at step 1.5 — BEFORE the crash drain.
static PRE_FLUSH: [AtomicPtr<()>; 8] = [const { AtomicPtr::new(null_mut()) }; 8];
pub fn register_pre_flush(f: extern "C" fn()) -> Result<(), PreFlushFull>;
```

A registrant's contract, asserted per registrant and **not** provable in general: **no allocation, no lock, one `write_all`, and it must not touch the `World`**. The profiler's `flush_on_panic` obeys it by moving its telemetry double buffer and file handle out of the `Profiler` `Resource` into a process-static — consistent with S12's extent rule, since both extents are compile-time constants.

**Eight slots is a hard cap; a ninth registration returns `Err` and emits `boyko-E0118`.** *(The seam record illustrated this as `E0110`. That number is taken: `W0110` is `OUT_LOCK`'s steal code, `DIAGNOSTICS` is dense with `index == code_idx`, and registry check 1 asserts numbers strictly increasing — two rows numbered 110 would not compile. `E0118` is the next free slot in the `01xx` band. Deviation recorded in §Seam disposition.)*

#### `sink_can_accept()` — one predicate closes both lifecycle holes

```rust
#[inline] fn sink_can_accept() -> bool;   // one Relaxed load of SINK_STATE
```

When it is **false** (`NotBooted` or `Exited`) **and** the level is `Warn`/`Error`, the record takes the **synchronous channel** instead of being dropped. That closes the pre-`boot()` hole and the symmetric post-`shutdown()` hole with one branch. Cost: one extra load plus a predicted-not-taken branch on the **failed-gate** path of `warn!`/`error!` only — `info!`/`debug!`/`trace!` are untouched, so the ≤ 3 ns row stands and the new `log_disabled_warn ≤ 4 ns` row (§Performance targets) is what bounds the addition.

**Deferred diagnostics.** Any condition observed *below* the logger or *before* it is `boyko_diag::raise(DiagFlag)` plus a counter; `boyko_ecs`'s fold reads `take_raised()` at the first drain after boot and emits the code then. So a profiling `W9201` refused before `LogPlugin::build` is not lost — it is emitted at frame 1. This is strictly better than "boot the logger earlier", which is unenforceable across every host.

**RED for all four** *(S5)*: (a) `warn!` before `boot()` ⇒ the bytes appear on the synchronous destination; restore the `.bss`-zero drop ⇒ no bytes ⇒ red. (b) a severe record after `shutdown()` ⇒ same. (c) a registered `PRE_FLUSH` callback sets a flag; panic ⇒ the flag is set **and** it ran *before* the crash drain; move the call after the drain ⇒ the ordering assertion reds. (d) a deferred `DiagFlag` raised pre-boot appears in frame 1's output; delete the sticky flag ⇒ absent ⇒ red.

### Decision 13: `fmtv` is deleted; `dsp!` formats in argument position *(fixes B4, and B2's second half)*
**What.** v1's `fmtv(&x)` ran user `Display::fmt` *while the ring tail held a partially-written record and `write` had not advanced*. A nested emit from that `Display` — or from anything it calls — would overwrite the outer record and publish one `len` for two interleaved payloads, decoded by the wrong function pointer. An unwind through the same window left the ring in the same state. The SAFETY clause "two producers on one lane is unrepresentable" is a proof about *threads* and does not touch this.

**Replacement — with the expansion form stated, because the naive one does not borrow-check** *(fixes F22)*. v2 described `dsp!` as "yielding `&str` from a caller-owned stack buffer" without saying which form. The block form `{ let mut buf = […]; &buf }` returns a reference to a local and does not compile. The form that works is a **by-value temporary in argument position**, whose `as_str()` borrow is extended to the end of the enclosing statement by Rust's temporary-lifetime rules for function-call arguments:

```rust
/// Renders `Display` into a stack buffer OWNED BY THE ARGUMENT EXPRESSION and
/// yields `&str`. Expands in ARGUMENT POSITION, so it runs to completion
/// BEFORE `emit_impl` is called and before any lane state is touched.
/// Overflow truncates and sets STR_TRUNCATED; it can never overrun a ring.
pub struct DspBuf<const N: usize> { buf: [u8; N], len: u16, truncated: bool }
impl<const N: usize> DspBuf<N> {
    #[inline] pub fn render(v: &impl core::fmt::Display) -> Self { /* fmt::Write into buf */ }
    #[inline] pub fn as_str(&self) -> &str { /* UTF-8 safe by construction of the writer */ }
}
#[macro_export] macro_rules! dsp {
    ($e:expr)             => { $crate::DspBuf::<256>::render(&$e).as_str() };
    ($e:expr, $n:literal) => { $crate::DspBuf::<$n>::render(&$e).as_str() };
}
// warn!(Render, codes::W2201, "material {} rejected", dsp!(mat_id));
```

Rust evaluates arguments before the call, and the `DspBuf` temporary created inside the argument expression lives to the end of the full statement, so the `&str` is valid for the whole emit. **Nothing between lane acquisition and the `Release` store can call user code**, because encoding operates on already-materialised POD and `&str`. Backed by `debug_assert!(!IN_EMIT.replace(true))` in `emit_impl` — the guard exists to catch a future violation, not because one is representable today.

**Trade-off, both halves stated.** A `Display` costs `core::fmt` at the call site — the honest price, legible in the source, which is precisely why v1's invisible version was worse. And the `DspBuf<256>` is constructed **by value**: worst case a 264-byte stack object the compiler is free to (and in practice does) construct in place. `dsp!(x, 32)` exists for the sites where 256 bytes of stack in a deep call is not acceptable, and the truncation is reported (`STR_TRUNCATED`) rather than silent.

### Decision 14: ONE byte per target owns level + sampling + sync routing; `LogFilter` is deleted *(fixes M14; extended by X1)*
v1 had `LogFilter { ceilings: [Level; 256], dirty: bool }` mirroring `CEILINGS`, synced by a hand-rolled flag — two sources of truth for one datum, with a public `set_target_level()` writing only one of them, so the next unrelated `dirty` flip would silently push a stale value over the live one. It also re-offended the "capability/state is not a bare bool" rule.

**Replacement.** `CEILINGS` is renamed **`CONTROL`** and its byte is packed:

```
bit  [0..2]  level      (Off | Error | Warn | Info | Debug | Trace)
bits [3..6]  sample shift k  (0 = every record; else deliver 1 in 2^k)   — Decision 20
bit  [7]     sync route      (format on the caller, write synchronously) — Decision 20
```

`CONTROL` is authoritative for all three. The UI, a console command and any system read and write it through `target_control(id)` / `set_target_control(id, ctl)` / `set_target_level(id, lvl)`, the last of which is a **CAS** so it preserves the sibling bit-fields. There is no ECS mirror, no `dirty`, and no sync system. Change detection is not needed because there is nothing to reconcile.

**Why one packed byte and not three arrays** *(X1)*. The three runtime knobs the game-facing audience needs are delivered **in the register the gate already loaded**: one `Relaxed` byte load, one `and`, one `cmp`. A parallel `SAMPLE_SHIFT` array would cost a second load and a second cache line on the *enabled* path. `.bss`-zero still means level `Off`, shift 0, sync off — the "unbooted is free and correct" property is untouched. This is gated, not assumed: `log_disabled_runtime` must stay **NOT RESOLVED** against the v2 single-level shape in the same sitting (G10d), and if it resolves, the packing is reverted rather than the target being raised.

### Decision 15: Target IDs are compile-time-unique, VALID BY CONSTRUCTION, and cut into three bands *(fixes M15, F15; extended by X2)*
v1 hand-assigned `id = $id:literal` per target with a boot collision check that could only fire if *both* colliders registered — and nothing forced registration, so an unregistered target still gated against `CONTROL[ID]` and never tripped.

**Band map** *(re-cut by X2)*:

| Band | IDs | Uniqueness proof |
|---|---|---|
| Engine | 0..=95 | one `targets! { … }` table; `const _: () = assert!(strictly_increasing)` ⇒ **collisions do not compile** |
| Downstream source | 96..=223 | `define_target!` + boot check `boyko-E0104` naming both colliders |
| **Dynamic** | 224..=255 | minted at runtime from a name (Decision 18); the mint is the uniqueness proof |

Registry check 7 asserts every `LogTarget` impl in the workspace resolves to a table row or a `define_target!` expansion. The honour system is confined to code we do not own, and that is stated.

**`TargetId` is valid by construction; the `pub` field is removed** *(fixes F15)*. v2 declared `pub struct TargetId(pub u16)` against `MAX_TARGETS = 256` with the bound checked only under `debug_assert!`, and made `target_level`/`set_target_level` public. That is either a panic in a hot-path indexing operation or `get_unchecked` **UB reachable from safe public API in release**, and v2 stated neither. v3:

```rust
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)]
pub struct TargetId(u16);        // PRIVATE field — the invariant is `.0 < MAX_TARGETS`
```

The only constructors are (a) `targets!`, (b) `define_target!` — both `const` and both carrying `const _: () = assert!(id < MAX_TARGETS)` — and (c) `register_dynamic_target`, which returns ids from the dynamic band only. There is therefore **no representable out-of-range `TargetId`**, and `CONTROL.get_unchecked(id.0 as usize)` carries a SAFETY comment naming that closed constructor set as the invariant.

**`TargetId::INVALID` is deleted; absence is `Option<TargetId>`.** A public in-band sentinel that indexes an array is the same hazard in a nicer coat. `register_dynamic_target` returns `Option<TargetId>`; a game that has not yet registered stores `None` and cannot call `dyn_info!` at all. This removes v3-draft's `UNREGISTERED_DROPPED` counter, which existed only to count an unreachable state — a counter for an impossible event is a gate that cannot fire.

`boyko_utils::TypeIntern` is **not** usable here, and the reason is **one**, not two: `ID` must be a `const` for gate (a) to fold, and a runtime-minted intern id is not a `const`. *(v3 gave a second reason — "`boyko_utils` depends on `boyko_log`, not the reverse" — which S2 strikes: `boyko_utils` stays a zero-dep leaf and gains no edge in either direction. The conclusion is unchanged; one of its two supports was false.)* Recorded so the next reader does not re-derive it.

---

## Key decisions — the scope extension (games as a first-class audience)

### Decision 16: What "as much data as possible" can and cannot mean here
The ask is real, and one common answer to it is wrong for this engine: **enlarging the ring does not raise the capture rate.** The ring's job is to absorb burstiness between a producer offering up to 66 M records·s⁻¹ (15 ns/record) and a consumer formatting at ~500 K·s⁻¹. Enlarging it moves the loss point later; it does not move the *ceiling*, which is `core::fmt` on the sink thread. Four mechanisms actually move the ceiling or make the loss honest, and each is a separate decision below:

1. **Do not format** — `BinarySink` writes `{site_id, tsc_delta, len, flags, payload}` and defers formatting to an offline decoder (Decision 22). This is the only change that moves the throughput ceiling, and it ships **with a revert clause** (G12c): if it does not measure ≥ 5× the text sink in the same sitting, L13b is reverted rather than justified.
2. **Emit less, on purpose, and say so** — per-target sampling (Decision 20), whose census status is `UNPROVEN(sampled)` so a sampled count can never be read as a total.
3. **Keep the loss count honest at session scale** — saturating counters, power-of-two `EveryN`, cursor-wrap correctness (Decision 21). A session is hours; every `u32` in the design was audited against that.
4. **Do not discard the beginning of the capture silently** — rotation reports what it deleted (`W0112`, E21), and `Rotation::NONE` stays the engine default so a bench cannot lose its own start.

**What this plan will not do:** promise a lossless capture. It promises that loss is *counted, attributed to a target, and rendered as a status a reader cannot mistake for a total* — and that promise is gated by G11 and P2 at session scale, not by a 300-frame argument.

### Decision 17: Per-target statistics are the game's read surface — and the census is where a vacuous gate goes to die *(status vocabulary unified by S8)*
`TARGET_STATS: [TargetStatCell; MAX_TARGETS]` (16 KiB `.bss`, one 64 B cell per target, written by the consumer role, readable by anyone) carries `delivered` / `dropped` / `sampled_out` / `sync_routed` as `u64`. `LogCensus` (a `Resource`, `VmColumn`-backed) is its ECS-visible snapshot, refreshed once per drain.

**One status vocabulary for both diagnostics subsystems** — `boyko_diag::LossStatus` (A3), so a reader who has learnt the tokens in one artifact has learnt them in the other:

| `LossStatus` | Meaning here |
|---|---|
| `Measured` | records were delivered; the counts are totals |
| `Unproven` | zero records — **never** `clean` |
| `UnprovenLossy` | `dropped > 0`; the counts are lower bounds |
| `UnprovenSampled` | the target's shift is non-zero; `delivered` is `1/2^k` of the truth |
| `UnprovenUnsunk` | **no `Active` sink's filter accepts this target** — a game enabled a category, saw nothing, and would have concluded "clean". `boyko-W0111` fires once |

*(v3 had a sixth, `dropped=SATURATED(>=4294967295)`. S8 widens the counters to `u64`; there is no ceiling state left to name, and a token the census could never let a reader **compare** stops existing. §Refuted records why the v3 rejection of `u64` does not survive.)*

**Type name vs printed form, stated once so the two spellings elsewhere in this document are not read as two vocabularies**: the *type* is `boyko_diag::LossStatus` with the variants above; the census's *rendered* form keeps the v3 text — `status=UNPROVEN`, `UNPROVEN(lossy)`, `UNPROVEN(sampled)`, `UNPROVEN(unsunk)`, `MEASURED` — because those strings are what a support ticket quotes and what a reader greps. Every `UNPROVEN(x)` appearing later in this document is the rendered form of the corresponding `LossStatus` variant.

`LogCensus.lossy` is the single bit a UI must read before rendering any count as a total, and `G15` gates that the bit exists and flips. **The single game-facing surface is one `Resource`** — `DiagCensus { log: LogCensus, prof: ProfCensus, lossy: bool }` (S8) — so a game asks one question about diagnostic completeness rather than two that can disagree.

### Decision 18: Dynamic targets — 32 slots, interned by name, and the cost of losing gate (a) is stated
A game or mod names a category from data (`"mod:acme_weapons"`, a script namespace, a save-file field). `register_dynamic_target(name, initial) -> Option<TargetId>` is **cold, setup-time and idempotent by name**: it hashes into `DYN_NAMES`, an open-addressed, insert-only, fixed-capacity table of 32 cache-line slots in `.bss`. Not a map: no rehash, no growth, no allocation, and **the emission path never touches it** — emission carries the `TargetId`, and the name is resolved by the sink.

Emission uses `dyn_info!(id, …)` / `dyn_warn!(id, code, …)`, which have **two** gates instead of three: `T::STATIC_CEILING` does not exist for a target that is not a type. The cost is real and is not smoothed over: a dynamic site cannot be compiled out per-target, only by `GLOBAL_CEILING`. The bench `log_dyn_disabled` bounds it at ≤ 4 ns, and **G8d turns the comparison into a claim that can be withdrawn**: if `log_dyn_disabled − log_disabled_runtime` does **not** resolve above the sitting's floor, then the per-target `const` ceiling's benefit is unproven on this box and Decision 2's claim about gate (a) is **struck from this document** rather than restated.

**Why 32 and not "unbounded"** — see open question 8. Every slot comes out of the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and `TARGET_STATS` are all sized by; past 256 those three arrays become two-level structures. 32 data-defined categories is a lot; needing more is a signal that the taxonomy belongs in source.

### Decision 19: Downstream code tables — the same macro, a different prefix, and a lazily-minted dense index
`codes!` is exported with a `prefix` parameter. A game invokes it once (`prefix = "acme"`, `doc_root = "docs/diag"`), gets its own `pub const` per code and its own `DiagInfo` table, and invokes `codes_tidy!(root = …, prefix = …)` to generate **the same eight checks over its own corpus** — because the engine's checks prove nothing about a game's registry, and that sentence is in G9's assertion message, not only in this document.

The `RATE` index must stay dense. Engine codes carry a compile-time `CodeIdx::Static(u16)`; downstream codes carry `CodeIdx::Dynamic(&'static AtomicU16)`, minted on first use with the reserve-then-publish protocol (`CAS UNASSIGNED→RESERVED`, `fetch_add` on `CODE_OCCUPANCY`, `store(Release)`), so 16 threads racing on one code produce exactly one index and leak none (G9). Cost on the downstream `Warn`/`Error` path only: one extra `Relaxed` load and one predicted-not-taken branch (~1 ns, measured by `downstream_code_warn` against the engine-code `warn!` in the same sitting). `CODE_OCCUPANCY` past 90 % emits `boyko-W0114`.

**What happens at 100 %, which v3 did not say** *(fixes M3)*. 512 slots are shared by the engine (whose own block map spans ~20 subsystems), every game table and every mod — and a modded title is the *expected* exhaustion case, on the game-facing path. The behaviour is defined, and the one thing it may never do is alias:

```rust
pub const CODE_IDX_EXHAUSTED: u16 = u16::MAX;   // a RESERVED sentinel, not an index
```

- The mint returns `CODE_IDX_EXHAUSTED`. It **never** wraps `fetch_add` into an occupied slot, because an aliased index silently applies **another code's** `EveryN`/`MinInterval` state — a rate policy secretly shared between two unrelated subsystems, which is worse than the loss it would be hiding.
- **The record is still delivered.** A `Warn`/`Error` is not lost because a table filled up; it is emitted with **`Every` semantics** (no rate policy applied), because the alternative is that the first symptom of exhaustion is silence.
- `boyko-E0115` fires **once**, naming the prefix and the code that could not be minted, and `LogStats.codes_unindexed` counts every subsequent unindexed emission. Both are printed by the census.

**G9 gains an exhaustion leg** (§Gates): fill `CODE_OCCUPANCY` to `MAX_CODES`, mint once more ⇒ the returned index is the sentinel, `E0115` fires exactly once, the record still arrives, and no two codes share a `RateSlot`. **Red state**: make the mint `fetch_add(1) % MAX_CODES` ⇒ two codes resolve to one slot ⇒ the distinct-rate-state assertion fails.

**Decision 7 is NOT relaxed for games**: `Warn`/`Error` still MUST carry a code, and a code is still a promise of a documented page. Data-defined *codes* are refused (§Refused) precisely because a data-defined code cannot have one.

### Decision 19b: `LogPod` — game types as arguments, encoded FIELD BY FIELD *(re-cut by B10)*

**The defect first.** v3 wrote `const POD_LEN: usize == size_of::<Self>()` and "the encode half is ours — a `copy_nonoverlapping` of `POD_LEN` bytes", with `fn fmt_pod(bytes: &[u8], …)` on the sink. `#[derive(LogPod)]` requires `#[repr(C)]` and all-`LogValue` fields, and that **admits padding**: `struct S { a: u8, b: u32 }` has three padding bytes whose contents are uninitialised. Copying `size_of::<Self>()` bytes copies them, and the sink then materialises a `&[u8]` over uninitialised memory — **UB regardless of whether `POD_LEN` is honest**, which also makes "round-trips byte-identically" undefined as a property. G9b named the padded struct only as the red state for a *lying* `POD_LEN`, so the correct implementation's own defect was uncovered.

**What (corrected).** The blanket byte copy is deleted. The trait requires an encoder:

```rust
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    /// SUM OF FIELD ENCODED LENGTHS — not `size_of::<Self>()`. Padding is
    /// never part of the record, so no uninitialised byte can reach a sink.
    const POD_LEN: usize;
    /// Writes EXACTLY `POD_LEN` initialised bytes at `dst`. This is the
    /// invariant a hand-written `unsafe impl` takes on; the derive discharges
    /// it mechanically.
    ///
    /// # Safety
    /// `dst` must be valid for `POD_LEN` writes.
    unsafe fn encode_pod(&self, dst: *mut u8);
    /// Runs on the SINK, over the `POD_LEN` bytes `encode_pod` wrote.
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
```

`#[derive(LogPod)]` in `boyko_macros` generates `encode_pod` as a **sequence of per-field `LogValue::encode` calls** — the derive already requires every field to be `LogValue`, so this needs no new capability — and generates

```rust
const _: () = assert!(<Self as LogPod>::POD_LEN == /* Σ field MAX_ENCODED_LEN */);
const _: () = assert!(<Self as LogPod>::POD_LEN <= MAX_RECORD_BYTES - HEADER_BYTES);
```

Fields whose `MAX_ENCODED_LEN` is `usize::MAX` (dynamic, i.e. `&str`) are **rejected by the derive** with a named error, which is what keeps `POD_LEN` a `const` and keeps the sum well-defined. `#[repr(C)]` is still required — not for the copy, which no longer exists, but so that field *order* in the generated encoder is the declared order and a reordering is a visible source change.

**Decision 13's structural property is untouched, and now for a better reason.** `encode_pod` is generated code over `LogValue::encode`, so what runs between lane acquisition and the `Release` store is still ours, still POD-only, still incapable of calling user `Display`. The user's `fmt_pod` runs on the **sink thread, from the staging arena, in the same position as `site.decode`**. Asserted, not argued: test 24 uses a `LogPod` whose `fmt_pod` sets a TLS flag and requires the flag to be **unset** at the `Release` store and set only during drain.

A hand-written `unsafe impl` is still allowed and carries the stated burden ("`encode_pod` writes exactly `POD_LEN` initialised bytes"). **G9b's subject changes** (§Gates): the red state is no longer "drop the `POD_LEN == size_of` assert" but "**replace the derived field-by-field encoder with a `copy_nonoverlapping` of `size_of::<Self>()`**" ⇒ the padded-struct Miri leg reports an uninitialised read. That is a red that responds to the defect v3 actually had. G9b still **cannot make an arbitrary hand impl safe**, and says so.

The `*_kv!` macros (`info_kv!(Combat, "hit", dmg = d, target = t)`) put field **names** in the `&'static LogSite`, which is cold and never touched on the emission path — so structured output costs the same as positional output on every hot path.

### Decision 20: Sampling and sync-routing — two bits of `CONTROL`, both default-off, both gated with a revert clause
**Sampling.** `k = (ctl >> 3) & 0x0F`; when `k != 0`, deliver 1 record in `2^k`. The counter is `SAMPLE_CTR[lane][target]`, a `u16` **written only by the lane's owner** (the row index *is* the lane index), with plain `Relaxed` load/store and **never an RMW** — so it inherits the `Lane` SAFETY block's single-writer clause verbatim and costs no lock prefix. Seeded at claim time with `(lane * 0x9E37)` so two lanes do not phase-lock.

**What sampling cannot claim**: that the capture is *representative*. `1/2^k` is **strided, not random**; a periodic emitter aliased to `2^k` yields a systematically biased capture. The census prints `sampling=1/N (strided, not random)`, `boyko-W0113` fires once per sampled target, and E23 states the residual. A footnote nobody reads is not a control; a line in the log is.

**Sync routing.** Bit 7 routes a target's records to the synchronous channel: format on the caller, `write_oracle_line`, count `sync_routed`. It serialises the frame — that is the *point*: it is the per-target opt-in for "this must leave the process before the next instruction", the only partial answer to a hard crash (E22).

**What sync routing costs, and what it cannot claim** *(corrected by B9)*. v3 wrote "~200+ ns … durable-on-write". Both halves needed work.
- **~200 ns is the uncontended, console-only figure.** With Decision 9c's durable fan-out the uncontended cost is one further `write_all` to the crash handle; **contended, the only bound is `OUT_LOCK`'s 50 ms acquire deadline**, after which the writer *steals* and the line may interleave with another synchronous line (E25). A mechanism whose reason to exist is integrity can therefore be interleaved under contention. That is the trade; it is not smoothed.
- **"Durable-on-write" means the bytes left the process, not that they reached the platter.** `sync_data()` is opt-in via `LogConfig.sync_durable` (default off) at ~0.1-10 ms per record, because a sync bit that also `fsync`ed would serialise the frame on the disk instead of on the format.
- In a profile with **no** synchronous destination the bit is inert, and the census reports the target as `UnprovenUnsunk` rather than letting a reader infer durability from a set bit.

Both branches are predicted-not-taken in every default configuration. **G10d decides whether sampling ships default-on**: `log_enabled_0args` must be NOT RESOLVED against the pre-L12 baseline; if it resolves, `log-sampling` becomes a default-off feature and the ≤ 15 ns row is annotated with the measured cost. The gate decides the rung's disposition; this document does not pre-decide it.

### Decision 21: The session-scale integer audit *(completed by M2 — v3's table omitted the field it had just created and every `BinarySink` quantity)*
A 300-frame bench cannot distinguish a correct counter from one that wraps in 65 seconds. Every integer is audited against an hours-long session. **Rows marked ✚ are the ones v3's "every integer was audited" sentence did not cover**, which is what made the claim unbacked precisely for the sink a shipping title runs for hours.

| Quantity | Width | Behaviour at the limit | Where |
|---|---|---|---|
| `LogLane::write` / `read` | `u32` byte cursors | **Wraps, correctly.** Every comparison is `wrapping_sub`, every index is `& MASK`, and `w − r ≤ LANE_BYTES ≪ 2³¹`, so the unsigned difference is unambiguous across a wrap. Wrap arrives in ~2.4 h at 500 KB·s⁻¹·lane | E17, test 19 |
| `dropped`, `dropped_bytes` | **`u64`** *(was `u32`+saturate; S8)* | Accumulate. ~8 800 years at 66 M·s⁻¹. The per-lane cell is plain `u64` (single-writer), folded into an `AtomicU64` with `fetch_sub(observed)` | E18, G11 |
| `RateSlot::count` | `u32` | Wraps; harmless **only because `EveryN(n)` is power-of-two** (X3) | Decision 8 |
| `LogStats.*`, `TargetStat.*` | `u64` | ~584 years at 1 G·s⁻¹ | — |
| `LogRing::head`, `arena_cursor` | `u32` | Wrap-correct; masked indices into fixed-capacity columns | test 20 |
| `LogRing::seq` | `u64` | The reader's cursor. Monotone, never wraps in any reachable session | Decision 26 |
| ✚ **`LogLine::seq_lo`** | **`u32`** | **Stores only the low half of `seq`, and the reconstruction rule is this**: for any line still in the ring, `seq = ring.seq − ((ring.seq_lo ⊖ line.seq_lo) as u32 as u64)`, where `⊖` is `wrapping_sub`. Unambiguous because the ring holds at most `LINE_CAP ≪ 2³¹` lines, so the low-half difference is always the true difference. **The high half is never stored and never needs to be**, which is why the ~2.4 h wrap at 500 K rec·s⁻¹ is not a truncation. `since(cursor)` with a `cursor` older than the oldest retained line starts at the oldest and reports the difference in `LogRingIter::skipped` | test 20 |
| ✚ **`BinaryRecord::tsc_delta`** | **`u32`** | Delta from the file's current **anchor**. A `u32` of raw ticks spans **1.4 s at 3 GHz**, so the sink re-emits an anchor record whenever the delta would exceed `u32::MAX` **or** every 1 s, whichever comes first — and unconditionally after a rotation. A missed anchor is a decode refusal, never a wrong timestamp | Decision 22, G12b |
| ✚ **`BinaryRecord::site_id`** | **`u16`** | Indexes `SITE_DICT`'s **4096** entries, so the width has 16× headroom over the table. The **table**, not the width, is the limit: on a full `SITE_DICT` the sink emits `boyko-W0116` once and writes an **inline site record** (file/line/fmt spelled out) instead of a dictionary reference, so no record is lost and no id is reused | Decision 22 |
| ✚ **`BinaryRecord::len` / `flags`** | `u16` / `u8` | `len ≤ MAX_RECORD_BYTES = 2048`, checked at runtime in every profile (E3); `flags` has 3 of 8 bits used | E3 |
| ✚ **File offset / rotation counter** | `u64` / `u8` | Offsets are `u64` (17 EB). `Rotation::keep` is `u8`, so at most 255 retained files; the rotation *sequence* number in the header is `u32` (~4 G rotations) | E21, G12b |
| ✚ **`clock_epoch_lo`** | `u8` in the header | Low 8 bits of `boyko_diag::clock_epoch()`. Reconstructed by the sink against the live epoch; at most one boundary can lie between producer and sink (Decision 11) | S4 |
| `tsc` | `u64` | ~195 years | E8 |

### Decision 22: `BinarySink` — the only mechanism that raises the ceiling, shipped with a revert clause
The sink writes `{site_id: u16, tsc_delta: u32, len: u16, flags: u8, clock_epoch_lo: u8, payload}` with **no formatting**; `site_id` comes from `SITE_DICT`, a consumer-role-only open-addressed `*const LogSite -> u16` table (4096 entries, 64 KiB `.bss`), with a dictionary record emitted on a `#[cold]` miss and `boyko-W0116` + an inline site record on a full table. `logdec` (a small bin) replays the dictionary and formats offline.

**Every width above is pinned HERE, not deferred to the format document** *(fixes M2)*. `docs/LOG-BINARY-FORMAT.md` owns the byte layout and `schema_version`; it does **not** own the session-scale argument, because deferring the widths is what let v3 claim "every integer was audited" while auditing none of these. The audit rows — including the **anchor cadence** that a 1.4 s `u32` delta forces — are in Decision 21. The decoder **refuses** a `schema_version` mismatch rather than best-efforting it. Every rotated file re-emits the anchor and the dictionary so it decodes standalone.

**Revert clause (G12c)**: the entire justification is throughput. If `sink_sustained_rate_binary` does not measure ≥ 5× `sink_sustained_rate` in the same sitting, **L13b is reverted**. A format whose only reason to exist is speed must show the speed.

### Decision 23: Runtime control with no restart, no lock, and no I/O on the caller's thread
- **Levels / sampling / sync**: a `CAS` on one `CONTROL` byte from any thread. `CONTROL_EPOCH` is a `Release` counter a UI polls to know it must repaint — an `O(1)` substitute for the change detection Principle 0's refused ECS route would have given (see §Refused). *(S11 naming, stated once: the static is `CONTROL_EPOCH_CTR` and the public accessor is `control_epoch()`; "`CONTROL_EPOCH`" elsewhere in this document names the datum, not a symbol, and it is **not** a clock epoch — `boyko_diag::clock_epoch()` is — nor a flush sequence, which is `FLUSH_SEQ`.)*
- **Sink state / filter / floor**: plain byte stores into `SinkSlot` from any thread. A sink acts on the filter it read at the top of its **current** drain, so a change lands within one drain — a stated property, pinned by G13, not hidden.
- **Sink lifecycle (open / close / retarget)**: goes through `SINK_REQ`, a 16-entry `.bss` ring written under `OUT_LOCK`, consumed by the sink thread. **No `open`, no allocation and no syscall ever runs on the requesting thread** — G13b proves it with the per-thread counting allocator. A full queue is `boyko-E0107`, never a silent drop. A channel was rejected: it is an allocation and usually a `Mutex`.
- **`apply_control_spec("net=debug/6!, ecs=off")`** parses a console/env/file spec, applies it with one `CONTROL_EPOCH` bump, leaves unnamed targets **bit-identical**, and rejects an unknown name with a coded error rather than ignoring it (test 30).

**Capability vs state, as the project rule requires**: a category *exists* because a `LogTarget` (or a dynamic registration) exists — structural. It is *on or off* by a bit in `CONTROL` — state. The rule's substance is honoured at the layer that can afford it; §Refused records why `CONTROL` is not an ECS column and what that costs.

### Decision 24: The crash drain CASes the CONSUMER ROLE, not a state that merely correlates with it *(fixes B5)*

**The defect first.** v3 attempted `SINK_STATE.compare_exchange(from, CrashDraining)` for `from ∈ {Exited, NotBooted, Manual}` and called those "the three states in which no sink thread can be inside a drain". Two of them are; **`Manual` is not**. `Manual` does not mean "no consumer is running" — it means the consumer is an *arbitrary user, CLI or test thread that may be inside `drain()` right now*. A panic on any other thread would then CAS `Manual → CrashDraining` and start a **second consumer** over the same lanes, `STAGE`, `SITE_DICT` and `SINK_OUT` — precisely what the `LogLane` SAFETY block forbids. v3's mitigation ("`Manual` documents `drain()` as single-caller and asserts it") constrains manual callers *to one another*, not the crash path, and G14 only tested `Running`, so the hole was untested by construction. It was also reachable in **production**, because `shipping-min` was the `Manual` + crash-sink profile.

**What (corrected).** The role itself is the CAS'd object:

```rust
/// 0 = free; otherwise an opaque token identifying the thread that currently
/// HOLDS THE CONSUMER ROLE. This is the object the Lane SAFETY block's clause 2
/// is about, so it is the object that is CAS'd — not a state that correlates
/// with it.
static DRAIN_OWNER: AtomicU64 = AtomicU64::new(0);
```

Every consumer claims it the same way, and there are exactly four:

| Claimant | When | On failure |
|---|---|---|
| the sink thread (`SinkMode::Thread`) | at the top of every drain pass | re-park; try next pass |
| `drain()` (`SinkMode::Manual`) | on entry | return `DrainResult::Busy` — **not** a `debug_assert`, because a second manual caller is a user error, not a bug in this crate |
| `log_drain_system` (`SinkMode::Scheduled`) | once per frame in `Last` | skip this frame; the records stay in the lanes |
| the crash drainer | panic-hook step 3 | **return without draining** |

The panic hook (chained ahead of the existing hook) writes the panic message synchronously, runs the `PRE_FLUSH` callbacks (step 1.5, S5), then `flush()`. If `flush()` cannot succeed it attempts `DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire)` **once**. Only on success does it run Algorithms C into the `CrashSink` (a file **opened at boot**, because opening a file inside a panic hook is its own failure mode) and emit `boyko-E0109`. On failure it returns: some other thread holds the role and displacing it would put two consumers on one lane. Termination is a single CAS and a bounded drain; **no wait is added**.

`SINK_STATE` keeps its lifecycle job (`NotBooted`/`Running`/`Exiting`/`Exited`/`Manual`/`Scheduled`) and loses its exclusivity job. `CrashDraining` is deleted as a `SINK_STATE` variant — the fact that a crash drain is in progress is `DRAIN_OWNER != 0`, and there is now one authority for one question.

**What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Stated in E22 and in G14's "cannot claim" column, with the partial mitigations named (the per-target sync bit **and its real durability bound**, `flush_interval_ms`, and a crash file that at least exists and carries the session header). It also cannot drain when another consumer is mid-pass; in that case the records that consumer has already staged are written by *it*, and the rest are lost — which is the honest outcome and is why the loss is counted rather than assumed away.

### Decision 25: `LogRuntimePreset` — five presets, and the compile axis is NOT one of its columns *(re-cut by S9; consumer discipline by B8)*

**Two axes, and v3 conflated them.** `GLOBAL_CEILING` and the lane count are **compile-time consts**; a struct chosen by the host at run time cannot deliver either, so v3's table promised something its own type could not do. S9 separates them:

- **The compile axis is one env var, `BOYKO_PROFILE`**, read by exactly one `build.rs` in the workspace — `crates/boyko_diag/build.rs`. It fixes `GLOBAL_CEILING`, `LANE_COUNT`, the profiler's tier and the `profiling-analysis` feature. **`crates/boyko_log/build.rs` is not created**, and neither is `crates/boyko_ecs/build.rs`; both consts are re-exported from `boyko_diag`.
- **The runtime axis is `LogRuntimePreset`** (v3 called it "the `LogConfig` profile"), which selects sinks, rotation, sampling, sink mode and census policy. It has **no `GLOBAL_CEILING` column**.

| `LogRuntimePreset` | Sinks | Rotation | Sampling | `SinkMode` | Census | Intended for |
|---|---|---|---|---|---|---|
| `Dev` | console + file | `Rotation::NONE` | off | `Thread` | `OnFlush` | engine work, benches, goldens |
| `Editor` | console + file | on | off | `Thread` | `Interval(10)` | long editor sessions |
| `Shipping` | binary + crash | on | opt-in | `Thread` | `OnShutdown` | a released title |
| `ShippingMin` | crash only | on | opt-in | **`Scheduled`** *(was `Manual` — B8)* | `OnShutdown` | a title that wants no **resident** diagnostics thread |
| `Off` | none | — | — | — | — | G2's leg |

**Which `BOYKO_PROFILE` supplies which const** (the table `boyko_diag/build.rs` owns; reproduced here because a reader of this document needs both halves in one place):

| `BOYKO_PROFILE` | `GLOBAL_CEILING` | `LANE_COUNT` | default `LogRuntimePreset` |
|---|---|---|---|
| `dev` (default) | `Trace` | 80 | `Dev` |
| `editor` | `Debug` | 80 | `Editor` |
| `shipping` | `Info` | 32 | `Shipping` |
| `shipping-min` | `Warn` | 32 | `ShippingMin` |
| `off` | `Off` | 0 | `Off` |

`BOYKO_LOG_MAX_LEVEL` survives **only** under `BOYKO_PROFILE=custom`; setting it beside a named profile is a `compile_error!`. **The default is a default, not a coupling**: a `shipping` build may select `LogRuntimePreset::Dev` at run time, and the header must make that legible — which is why it prints **three independent facts**, `build_profile=… runtime_preset=… ceiling=…`, and not one profile name. The 128-bit **`boyko_diag::SessionId`** (one mint, shared with the profiler's artifact header, S11) appears beside them, so an uploaded log and an uploaded artifact identify the same session.

**`ShippingMin` has a consumer** *(B8)*. `SinkMode::Scheduled` puts the drain in `log_drain_system` under `DRAIN_OWNER`, once per frame, on the frame thread (Decision 10). What the profile actually buys is **no resident diagnostics *thread***, which is what the owner asked for; what it costs is a bounded per-frame drain and a hole around boot and shutdown, both stated in Decision 10. *(An owner-facing SCOPE question remains and is recorded in §Open questions: the profiler's `Always` tier still writes a telemetry stream synchronously on the dispatcher in this profile, so a title that chose `shipping-min` to avoid a resident diagnostics thread still pays a per-window `write_all`.)*

**CI legs: 5**, one per named profile, of which `dev` is the existing leg ⇒ **4 net new full-workspace builds**. `custom` is never built in CI. G16's cross-profile symbol census is a CI *step* consuming two legs' artifacts, not a sixth leg. Each leg needs its own `CARGO_TARGET_DIR` with a size cap and the standing "never two bench jobs concurrently" rule (`target/` once reached 74 GB and took the disk to zero, masquerading as mingw errors).

**G16** is a two-sided symbol gate: no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture may appear in the `shipping` binary, and it **must** appear in `dev`. Its fixture includes a `dyn_debug!` site, because a dynamic site has no gate (a) and `GLOBAL_CEILING` is the only thing that deletes it. **Second red** *(S9)*: `BOYKO_PROFILE=shipping BOYKO_LOG_MAX_LEVEL=trace cargo build` must fail with a named message; delete the `compile_error!` ⇒ it builds and the header prints a ceiling the profile does not name ⇒ red. **Third**: assert the header carries all three fields and that `runtime_preset` and `build_profile` **can differ** in one binary; print only one ⇒ red.

### Decision 26: In-frame consumption — and the handoff is a SPECIFIED structure, not a word *(fixes B2)*

`LogRing::since(cursor, &RingFilter) -> LogRingIter` returns records delivered since a monotone `seq`, oldest first, with `LogRingIter::skipped` reporting how many the ring wrapped past — **a console cannot silently miss lines**. The ring is fed by `log_drain_system` in `Last`, never from the emission path: **G15b reds if a record is visible before the drain that consumed it**, which is also what keeps the hot path from touching ECS storage.

**The transport, which v3 named three times and defined nowhere.** v3 wrote "push formatted lines to the ECS handoff ring" (§Algorithms C), "fed by `log_drain_system` in `Last`, from the sink's handoff" (here) and referenced it again in §Public API — with **no type, no capacity, no ordering, no overflow accounting, no budget row and no `Send`/`Sync` argument**. Every claim about the reader rests on it, and an undefined cross-thread queue is exactly the object this campaign's defects live in.

```rust
/// SPSC byte ring carrying FORMATTED LINES from the consumer role to the ECS.
/// Deliberately the SAME SHAPE as `LogLane` — same header, same MASK
/// arithmetic, same PAD/wrap rule, same Release/Acquire cursor pair — so it
/// introduces NO new protocol, only a new instance. Reusing the rule is why
/// its correctness argument is one sentence instead of a section.
#[repr(C, align(64))]
pub(crate) struct HandoffRing {
    write: AtomicU32, _p0: [u8; 60],                 // producer line: the consumer role
    read:  AtomicU32, _p1: [u8; 60],                 // consumer line: log_drain_system
    lost:  AtomicU64, lost_bytes: AtomicU64, _p2: [u8; 48],   // LossClass::Sink
    buf:   UnsafeCell<[MaybeUninit<u8>; HANDOFF_BYTES]>,
}
static ECS_HANDOFF: HandoffRing = HandoffRing::NEW;
pub const HANDOFF_BYTES: usize = if SHIPPING { 64 * 1024 } else { 256 * 1024 };
const _: () = assert!(HANDOFF_BYTES.is_power_of_two());
```

| Property | Value | Why |
|---|---|---|
| **Producer** | whoever holds `DRAIN_OWNER` — the sink thread, `drain()`, `log_drain_system` under `Scheduled`, or the crash drainer. **Exactly one at a time, by the Decision 24 CAS** | this is what makes the ring genuinely SP, rather than "SP by convention" |
| **Consumer** | `log_drain_system` **only**, holding `ResMut<LogRing>` — so the scheduler's exclusivity analysis is the SC proof | one writer of `LogRing`, which is what B1's `Send`/`Sync` argument needs |
| **Capacity** | 256 KiB `dev` / 64 KiB `shipping` (Decision 3's matrix). At ~120 B per formatted line that is ~2 200 / ~550 lines per frame | a frame that formats more than that is already lossy at the sink |
| **Ordering** | `write.store(Release)` publishes; `read.store(Release)` after the ECS copy frees. Identical to `LogLane` | the payload here is plain text with no pointers, so the provenance clause does not apply |
| **Overflow** | the producer **refuses**, counts into `lost`/`lost_bytes` as `boyko_diag::LossClass::Sink`, and the drain emits **one `boyko-W0117`** per drain carrying the count. `LogCensus.lossy` is set. **Never silent** | the byte sinks already have the record; only the *in-frame view* is short, and a reader must be able to tell |
| **Allocation** | **none, ever** — `.bss`, compile-time extent, S12's rule | |
| **Presence** | only when `LogConfig.ecs_ring` is set. Absent in `ShippingMin` | no ECS reader, no ring |

```
// SAFETY (manual Sync for HandoffRing):
//   1. WRITE side: only the holder of DRAIN_OWNER writes `buf`/`write`. The
//      role is a single CAS'd token (Decision 24), so two producers is
//      unrepresentable — the same clause LogLane's SAFETY block makes about
//      `owner`, one level up.
//   2. READ side: only `log_drain_system` reads `buf` and writes `read`, and
//      the scheduler grants it `ResMut<LogRing>` exclusively, so two consumers
//      is unrepresentable.
//   3. Visibility: bytes written before `write.store(Release)` are visible to
//      the reader's `Acquire`; the reader never reads past its observed `w`
//      and never advances `read` over bytes it has not copied into `LogRing`.
//   4. No pointers cross: the payload is UTF-8 bytes already formatted by the
//      consumer role, so the `LogLane` provenance clause has no analogue here.
```

**The stated bound** is "sink park interval + one frame" (≤ 2 frames in practice) under `Thread`, and **one frame** under `Scheduled` (the drain and the ECS copy are the same system). G15 cannot claim tighter. A per-frame **`frame_epoch` record** *(renamed from `EPOCH` — S11, three meanings collided)* lets a reader attribute every record to exactly one frame; a record emitted *during* the drain is attributed to the next frame, and test 29 asserts that rather than assuming it.

---

## Audience conflicts — named, decided, and costed for the losing side

Five places where the engine's needs and a game's needs genuinely pull in opposite directions. Each is decided; each records what the losing side gives up.

| # | Conflict | Decision | **Cost to the losing side** |
|---|---|---|---|
| **C1** | **Compile-time categories** (three gates, one const-folded per target, zero cost when off) vs **data-defined categories** (a mod's name is not a Rust type) | **Both, in separate bands.** Static targets keep all three gates; dynamic targets (224..=255) have two | A dynamic site **cannot be compiled out per-target** — only `GLOBAL_CEILING` deletes it. Bounded at ≤ 4 ns disabled, and **G8d can strike the claim that gate (a) matters at all** if the difference does not resolve on this box |
| **C2** | **"As much data as possible"** vs **a fixed-capacity ring that drops** | The ring stays fixed; the answer to volume is **not formatting** (`BinarySink`) plus **honest loss accounting at session scale** | A game cannot have a lossless capture from this crate. It gets: a counted, per-target, **`u64`** loss report classified by `boyko_diag::LossClass` and a `lossy` bit; and `sink_sustained_rate`'s knee measured, not asserted. Enlarging the ring is explicitly refused (§Refused) because it moves the loss point without moving the ceiling |
| **C3** | **Runtime toggling from a console / dev menu / remote** vs **no lock and one authority on the hot path** | One **packed `CONTROL` byte**, CAS-written from any thread, read `Relaxed` by the gate. Sink lifecycle goes through `SINK_REQ`; no I/O on the caller | No `Query`, no change detection, no `EnableTag` over `CONTROL` — it is not an ECS column, and cannot be (Principle 0 exception, Invariant 7). Mitigation is `CONTROL_EPOCH`, a poll not a subscription. Filter changes land within one drain, not instantly (G13's stated limit) |
| **C4** | **What ships in a released title** (small, quiet, crash-durable) vs **what the engine needs** (loud, complete, byte-frozen) | **One compile axis (`BOYKO_PROFILE`, five values) + `LogRuntimePreset`** (Decision 25, S9), with `build_profile` / `runtime_preset` / `ceiling` printed as three independent facts | `shipping` compiles out `debug!`/`trace!` entirely — a support ticket cannot ask a player to "turn on trace" without a different build. **`shipping-min` runs its drain on the frame thread** (`SinkMode::Scheduled`), so it has no in-frame HUD, a bounded per-frame drain cost, and a stated hole around boot/shutdown where only the synchronous channel and the crash drain apply (Decision 10). *(v3 gave it `Manual`, i.e. **no** consumer, which made the crash file structurally contain the session's beginning — B8.)* |
| **C5** | **Gameplay reading its own diagnostics in-frame** vs **the drain staying off the frame thread** | `LogRing` is fed by `log_drain_system` in `Last` from **`ECS_HANDOFF`**, a specified SPSC byte ring (Decision 26); the reader is a cursor + filter | The reader is **one drain + one frame behind** (one frame under `Scheduled`), the handoff can itself drop under a formatting storm — counted as `LossClass::Sink`, reported by `W0117`, `lossy` set — and gameplay **may not branch on log counters** (§Refused: they are lower bounds under drop, schedule-dependent, and break replay). Display and telemetry only |

---

## Data structures

```rust
// ─────────────────────────── boyko_log/src/level.rs ───────────────────────────

/// Severity. Lower value = more severe. `Off` is a THRESHOLD value only; no
/// record is ever emitted at `Off`, which is why `.bss`-zero == "all disabled".
#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level { Off = 0, Error = 1, Warn = 2, Info = 3, Debug = 4, Trace = 5 }
```
No `Fatal`: a fatal condition panics through an existing `#[cold] fn … -> !` helper and the panic hook flushes. Two spellings of "die" is one too many.

```rust
// ─────────────────────────── boyko_log/src/site.rs ────────────────────────────

/// Immutable per-call-site metadata. One `'static` per macro expansion,
/// referenced by pointer, so the record pays 8 B instead of re-carrying
/// file/line/fmt/code. Dereferenced ONLY by the sink.
pub struct LogSite {
    pub target:   TargetId,     // 2
    pub level:    Level,        // 1
    pub class:    u8,           // 1  b'B' | b'E' | b'W' | 0
    pub code:     u16,          // 2  printed number; 0 when the level has none
    pub code_idx: CodeIdx,      // 4  DENSE registry index (M12); Static or Dynamic (D19)
    pub line:     u32,          // 4
    pub file:     &'static str, // 16
    pub fmt:      &'static str, // 16 the format literal, printed by the sink
    /// Field names for the `*_kv!` forms; empty for the positional forms.
    /// `&'static`, cold, NEVER touched on the emission path — which is why
    /// structured output costs the same as positional output (D19b).
    pub fields:   &'static [&'static str],
    pub prefix:   &'static str, // "boyko" for the engine; a game declares its own (D19)
    /// Monomorphised per ARGUMENT-TUPLE type; identical tuples share one
    /// instantiation. Cold: called on the sink thread only.
    pub decode:   unsafe fn(*const u8, usize, &mut LogFormatter),
}
// The per-SITE `Once` latch is NOT a field here: `LogSite` is immutable
// `&'static` data. The macro expands a sibling `static FIRED: AtomicBool`
// beside each `Once` site — per-site, private line, 1 byte (Decision 8, F11).

// ────────────────────────── boyko_log/src/record.rs ───────────────────────────

/// 20 B, PACKED — the ring is byte-oriented and records are never aligned, so
/// alignment padding would be pure waste. `code`, `level` and `lane` are NOT
/// duplicated here (v1 did): the sink holds the site pointer and knows the lane
/// it is draining, so three bytes bought nothing (N32).
///
/// `site` is a REAL pointer field so provenance round-trips; a null `site` is
/// the PAD sentinel (§Algorithms A5).
#[repr(C, packed)]
struct RecordHeader {
    site:  *const LogSite, // 8
    tsc:   u64,            // 8  raw boyko_diag::clock::ticks(); scaled by the sink
    len:   u16,            // 2  TOTAL record bytes incl. header — the walk step
    flags: u8,             // 1  STR_TRUNCATED | SUPPRESSED_FOLLOWS | TOO_LARGE
    /// Low 8 bits of `boyko_diag::clock_epoch()` (S4). SPENDS v3's `_pad`, so
    /// the header does NOT grow and the 20 B assert below is unchanged. Read as
    /// a register-resident global, not probed, so the <= 15 ns row is untouched.
    /// 8 bits suffice: the sink is at most one park interval behind the
    /// producer, so at most one epoch boundary can lie between them and the
    /// full u32 is reconstructed against the live epoch. The sink RENDERS it
    /// beside every timestamp, so a record straddling a suspend is legible
    /// rather than merely wrong.
    clock_epoch_lo: u8,    // 1
}
const HEADER_BYTES: usize = 20;
const _: () = assert!(core::mem::size_of::<RecordHeader>() == HEADER_BYTES);

/// Hard cap, checked at RUNTIME against `encoded_len` — not asserted as
/// unreachable. Five 256-byte `&str`s exceed 1 KiB, and the plan allows
/// 12-element tuples, so v1's "unreachable, debug_assert'ed" described a
/// debug-build panic reachable from safe user code (N29). Over-cap records are
/// DROPPED with the `TOO_LARGE` flag and counted, in every profile.
const MAX_RECORD_BYTES: usize = 2048;
const MAX_STR_BYTES:    usize = 256;

// ─────────────────────────── boyko_log/src/lane.rs ────────────────────────────

/// One SPSC byte ring. THREE cache-line partitions, not two: statistics are
/// written by the producer AND cleared by the consumer, so they are a third
/// partition — putting them on the producer line (v1) meant the consumer
/// RMW'd the producer's hot line on every drain, most often precisely during
/// the drop storm the counters exist to measure (M9).
///
/// RENAMED from `Lane` to `LogLane` (S11): the lane *identity* is now
/// `boyko_diag`'s and one name may not mean two things. There is no `owner`
/// field any more — the index comes from `boyko_diag::lane()`, which is
/// unique per live thread by construction (S3).
#[repr(C, align(64))]
pub(crate) struct LogLane {
    // ── line 0: PRODUCER-owned ───────────────────────────────────────────────
    /// Absolute WRAPPING byte counter; `off = write & MASK`. `Release`-stored;
    /// this store is the happens-before edge that publishes the payload.
    /// Wrap at 2^32 bytes (~2.4 h at 500 KB/s) is CORRECT, not merely unlikely:
    /// every comparison is an unsigned wrapping difference and every index is
    /// masked (Decision 21, E17, test 19).
    write:        AtomicU32,   // 4
    /// Producer's private cache of `read`; refreshed (Acquire) only when the
    /// cached value says "full". This half is what buys throughput; padding
    /// alone measured SLOWER (Decision 4).
    read_cached:  Cell<u32>,   // 4
    _pad0:        [u8; 56],

    // ── line 1: CONSUMER-owned ───────────────────────────────────────────────
    read:         AtomicU32,   // 4  Release-stored AFTER staging (B1)
    write_cached: Cell<u32>,   // 4
    _pad1:        [u8; 56],

    // ── line 2: LANE-OWNED STATISTICS (producer writes, consumer folds) ──────
    /// `boyko_diag::LossCell` — plain `u64` load/store BY THE LANE OWNER, with
    /// NO lock prefix and no RMW (S8). v3 used saturating `AtomicU32` on the
    /// argument that an 8-byte RMW costs more; on x86-64 `lock xadd` costs the
    /// same at 4 and 8 bytes, and a single-writer cell needs no RMW at all —
    /// so the rejection does not survive, and with `u64` the `SATURATED` token
    /// (which a reader could never COMPARE) stops existing.
    /// The consumer folds into a `LossTotal` with `fetch_sub(observed)`, never
    /// `store(0)`: a `store` loses any increment landing between load and clear.
    loss:          [LossCell; LOSS_CLASSES],   // Overflow | Unclaimed | Refused | Sink
    /// Deliberately NOT emitted (Decision 20). NOT a loss, so NOT a LossClass —
    /// counted separately so conflating the two cannot make either number a
    /// liar. The property `emitted == drained + dropped + sampled_out` depends
    /// on the separation being exact.
    sampled_out:   AtomicU64,
    _pad2:         [u8; PAD2],

    // ── payload ──────────────────────────────────────────────────────────────
    buf: UnsafeCell<[MaybeUninit<u8>; LANE_BYTES]>,
}
const _: () = assert!(core::mem::align_of::<LogLane>() == 64);
const _: () = assert!(core::mem::offset_of!(LogLane, read) == 64);
const _: () = assert!(core::mem::offset_of!(LogLane, loss) == 128,
    "statistics are a third partition: producer writes, consumer folds");

// SAFETY (manual Sync for LogLane):
//   1. WRITE side: exactly one thread ever writes `buf` or `write` — the one
//      whose `boyko_diag::lane()` returns this index. The uniqueness is the
//      SUBSTRATE's, and it is single-writer by construction on both of its
//      two paths (S3): a pool worker's index IS its dense `worker_id`
//      (`boyko_threadpool::worker::worker_main`, one live thread per id by the
//      pool's own construction), and every other thread's index comes from
//      `claim_lane()`'s load-then-CAS over the spare slots, whose loser
//      retries a different slot. No two LIVE threads hold one index, so two
//      PRODUCER THREADS on one lane is unrepresentable.
//      NOTE what changed from v3: the exclusivity used to be argued from a
//      CAS on a field of THIS struct. It is now argued from a CAS in
//      `boyko_diag`. That is not a weakening — it is the same argument with
//      ONE owner instead of two, which is the point: two registries would let
//      the profiler hand a reclaimed index to a new thread while this crate
//      still believed the old one owned it.
//   1b. Re-entrant emit on ONE thread is separately excluded: no user code can
//      run between lane acquisition and the `Release` store, because `dsp!`
//      runs in argument position and encoding operates on POD and `&str` only
//      (Decision 13). `debug_assert`ed by IN_EMIT.
//   1c. `LogPod` does not weaken 1b: the ENCODE half is our blanket
//      `copy_nonoverlapping` of POD bytes, and the user-supplied `fmt_pod`
//      runs on the SINK thread from the staging arena, in the same position as
//      `site.decode` (Decision 19b, asserted by test 24).
//   1d. `SAMPLE_CTR[lane]` is written only by the lane's owner (the ROW INDEX
//      IS THE LANE INDEX), with Relaxed load/store and never an RMW, so it
//      inherits clause 1 verbatim (Decision 20).
//   1e. `read_cached` is a `Cell<u32>` READ AND WRITTEN ONLY BY THE PRODUCER
//      that owns the lane — it is the reason `Lane` is not `Sync` by
//      derivation, and clause 1's exclusive-write argument covers it exactly:
//      a second producer thread cannot exist, and the consumer never names it.
//      Its value is a STALE lower bound on `read`; staleness is safe because
//      it can only make the producer refuse space it actually had, never grant
//      space it did not (Decision 5's induction) (F23).
//   1f. `write_cached` is the mirror clause: `Cell<u32>`, read and written
//      only by the thread currently holding the CONSUMER role (clause 2), and
//      a stale value can only make the consumer drain less than was available,
//      never read past `write` (F23).
//   2. READ side: exactly one thread reads `buf` and writes `read` — the
//      holder of `DRAIN_OWNER`, a single CAS'd token (Decision 24, B5).
//      All FOUR consumers claim it identically: the sink thread, `drain()`
//      under `Manual`, `log_drain_system` under `Scheduled`, and the crash
//      drainer. v3 instead CAS'd `SINK_STATE` out of {Exited, NotBooted,
//      Manual} and called all three quiescent; `Manual` is NOT — it means an
//      arbitrary thread may be inside `drain()` right now, so a panic
//      elsewhere started a SECOND consumer over these very bytes. CASing the
//      role removes the gap between "a state that correlates with exclusivity"
//      and "exclusivity".
//   3. Payload visibility: bytes written before `write.store(_, Release)` are
//      visible to a thread observing that value via `Acquire`. The consumer
//      never reads past its observed `w`, AND never advances `read` over bytes
//      it has not yet copied out (Algorithms C).
//   4. Retire: `boyko_diag::release_lane()` marks the SUBSTRATE's slot
//      RETIRING, on the producer thread, after its last write. The consumer
//      calls `boyko_diag::reclaim(i)` only after observing RETIRING and
//      `read == write` for THIS lane, so no producer write can follow a
//      reclaim. The state lives in one place, so the profiler cannot reissue
//      the index while this crate still holds undrained bytes.
unsafe impl Sync for LogLane {}

/// NOT a constant of this crate any more (S3): 80 in `dev`/`editor`, 32 in
/// `shipping`/`shipping-min`, from `BOYKO_PROFILE` via `boyko_diag/build.rs`.
/// v3's `MAX_LANES = 128` is deleted, along with its `option_env!` override.
pub(crate) use boyko_diag::LANE_COUNT;
pub(crate) const LANE_BYTES: usize = 16 * 1024;  // power of two: MASK arithmetic
const MASK:          u32   = (LANE_BYTES - 1) as u32;
/// Usable span. ONE slot reserved so `used == CAPACITY` cannot be confused with
/// `used == 0` without a third variable — and, critically, so that
/// `avail = CAPACITY - used` in Algorithms A6 cannot underflow (F6).
const CAPACITY:      u32   = (LANE_BYTES - 1) as u32;
const ERROR_RESERVE: u32   = (LANE_BYTES / 8) as u32;   // 2 KiB, Error-only tail
const _: () = assert!(LANE_BYTES.is_power_of_two());
const _: () = assert!(ERROR_RESERVE < CAPACITY);        // else no non-Error record ever fits

pub const LANE_ARRAY_LEN: usize =
    if (GLOBAL_CEILING as u8) == 0 { 0 } else { LANE_COUNT as usize };
static LOG_LANES: [LogLane; LANE_ARRAY_LEN] = [LogLane::NEW; LANE_ARRAY_LEN];

// ────────────────────── boyko_log/src/sink/ecs.rs (B2) ────────────────────────
// `HandoffRing` / `ECS_HANDOFF` — the sink -> ECS transport, specified in full
// in Decision 26 (layout, capacity, ordering, overflow, budget row, SAFETY).
// Same shape and same wrap rule as `LogLane`: no new protocol, one new
// instance. Single producer = the DRAIN_OWNER holder; single consumer =
// `log_drain_system` holding `ResMut<LogRing>`.

// ────────────────────────── boyko_log/src/target.rs ───────────────────────────

/// PRIVATE field: the invariant `.0 < MAX_TARGETS` is upheld by a CLOSED set of
/// three constructors (`targets!`, `define_target!`, `register_dynamic_target`),
/// which is what makes the hot-path `get_unchecked` sound. v2's `pub u16` made
/// out-of-range values constructible from safe code (F15). There is no
/// `INVALID` sentinel — absence is `Option<TargetId>`.
#[repr(transparent)] #[derive(Clone, Copy, PartialEq, Eq)] pub struct TargetId(u16);

pub trait LogTarget: 'static {
    const NAME: &'static str;
    const ID: TargetId;           // const: required for gate folding
    const STATIC_CEILING: Level;  // const: Unreal's compile-time ceiling
}
pub const MAX_TARGETS:    usize = 256;
pub const DYN_BAND_START: usize = 224;                          // Decision 15's re-cut band
pub const DYN_BAND_LEN:   usize = MAX_TARGETS - DYN_BAND_START; // 32

/// ONE packed byte per target: level | sample shift | sync route (Decision 14).
/// One `Relaxed` byte load + `and` + `cmp` per enabled-check. 256 B; one line.
/// `.bss`-zero == level `Off`, shift 0, sync off == disabled until boot arms it.
static CONTROL: [AtomicU8; MAX_TARGETS] = [const { AtomicU8::new(0) }; MAX_TARGETS];
/// Monotone; `Release`-added on every control change. A UI POLLS this to know
/// it must repaint — the O(1) stand-in for the change detection the refused ECS
/// route would have given (§Refused, Decision 23). RENAMED from `CONTROL_EPOCH`
/// (S11): three unrelated things were called "epoch" across the two plans, and
/// this one is the control-change counter, not a clock epoch and not a flush
/// sequence.
static CONTROL_EPOCH_CTR: AtomicU32 = AtomicU32::new(0);   // read via control_epoch()

/// ONE pointer publishes name+len together — v1's `AtomicPtr<u8>` lost the
/// length of a `&'static str` (N28). Read by the sink and by `targets()` only.
pub struct TargetInfo { pub name: &'static str }
static TARGETS: [AtomicPtr<TargetInfo>; MAX_TARGETS] = ...;

/// Dynamic-target name interning. Open-addressed, insert-only, fixed capacity,
/// one cache line per slot. NOT a map: no rehash, no growth, no allocation, and
/// the emission path never touches it (Decision 18). 2 KiB .bss.
#[repr(C, align(64))]
struct DynSlot { hash: AtomicU64, len: AtomicU8, bytes: UnsafeCell<[u8; 47]> }
static DYN_NAMES: [DynSlot; DYN_BAND_LEN];
// SAFETY: `bytes`/`len` are written before `hash.store(h, Release)`; a reader
// that observes a non-zero hash via Acquire observes the completed name. A
// slot's hash transitions 0 -> h exactly once, by CAS. No slot is ever reused,
// so a published name is immutable for the process lifetime.

/// Sink-written, anyone-readable. NOT a mirror of anything: it is the only
/// place delivered-per-target counts exist (Decision 17). 16 KiB .bss.
#[repr(C, align(64))]
struct TargetStatCell { delivered: AtomicU64, dropped: AtomicU64,
                        sampled_out: AtomicU64, sync_routed: AtomicU64,
                        _pad: [u8; 32] }
static TARGET_STATS: [TargetStatCell; MAX_TARGETS];

/// One u16 row per LANE, one column per target. Written ONLY by the lane's
/// owner with plain Relaxed load/store, never an RMW (Decision 20, SAFETY 1d).
/// Row count follows `boyko_diag::LANE_COUNT` (S3): 40 KiB in `dev`, 16 KiB in
/// `shipping` (v3: 64 KiB at 128 lanes).
static SAMPLE_CTR: [[Cell<u16>; MAX_TARGETS]; LANE_COUNT as usize];

// ────────────────────────── boyko_log/src/codes.rs ────────────────────────────

#[repr(C)] #[derive(Clone, Copy)] pub struct WarnCode  { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct ErrorCode { num: u16, idx: CodeIdx }
#[repr(C)] #[derive(Clone, Copy)] pub struct PanicCode { num: u16, idx: CodeIdx }
// Distinct newtypes ⇒ `warn!(T, codes::E2101, ..)` does not compile.

/// Engine codes carry a compile-time dense index; downstream codes carry a
/// pointer to a lazily-minted cell (Decision 19). Cost: one extra Relaxed load
/// and one predicted-not-taken branch, on the DOWNSTREAM Warn/Error path only.
#[derive(Clone, Copy)]
pub enum CodeIdx { Static(u16), Dynamic(&'static AtomicU16) }

#[repr(u8)] pub enum RatePolicy { Every, Once, OnceCounted, EveryN(u16), MinIntervalMs(u16) }
// `codes!` emits `const _: () = assert!(n.is_power_of_two())` for EveryN, so
// `count & (n-1)` is exact across a u32 wrap (Decision 8, Decision 21).
// `Once`/`OnceCounted` do NOT use `RATE` at all — the latch is per SITE (F11).

/// Registry ROW STATUS — the mechanism that lets L2 commit alone on a
/// grandfathered corpus (Decision 6, F20). `Pending` rows must have ZERO
/// emitters (check 3b); `Live` rows must have at least one (check 3) AND a doc
/// page (check 2, `Live`-only per S6). `Historical` (B6) is for a code that
/// exists only in a frozen artifact this repository will not edit: zero
/// emitters, NO doc page required, never becomes `Live`, excluded from check
/// 3c's migration count.
#[derive(Clone, Copy, PartialEq, Eq)] pub enum CodeStatus { Live, Pending, Historical }

pub struct DiagInfo {
    pub number: u16, pub class: u8,
    pub prefix: &'static str,    // "boyko" for the engine; games declare their own
    pub summary: &'static str,   // one line, embedded, printable from a message
    pub rate: RatePolicy,
    pub status: CodeStatus,
    pub doc: &'static str,       // "docs/diagnostics/W1501.md" — check 2's target
}
static DIAGNOSTICS: [DiagInfo; N];       // dense, sorted; index == code_idx
const MAX_CODES: usize = 512;
static CODE_OCCUPANCY: AtomicU16;        // downstream minting; W0114 at 90 %
/// RESERVED sentinel, never an index. Returned when the 512-slot space is
/// exhausted; the record is still delivered, with `Every` semantics and no
/// rate state, and `boyko-E0115` fires once (M3). It is NEVER an aliased
/// index, because aliasing silently applies another code's EveryN/MinInterval
/// state to an unrelated subsystem.
pub const CODE_IDX_EXHAUSTED: u16 = u16::MAX;

/// 64 B — one code per cache line. v1's 16 B slot false-shared four unrelated
/// subsystems' codes on one line (Decision 8). `fired` is GONE (M1): it was
/// dead from the moment `Once`/`OnceCounted` stopped using `RATE` at all, and
/// the census line that read it printed a literal rather than an observation.
#[repr(C, align(64))]
struct RateSlot { count: AtomicU32, last_tsc: AtomicU64, suppressed: AtomicU32, _pad: [u8; 44] }
static RATE: [RateSlot; MAX_CODES];      // 32 KiB .bss

/// The per-SITE `Once` latch, and the ENUMERATION that makes its census row a
/// real observation (M1). The node IS the static the macro already expands
/// beside each `Once` call site; the list only adds `next`. Pushed by a
/// `#[cold]` CAS loop on the site's SINGLE fire — the same branch that already
/// performs the one `FIRED.swap(true)` — so nothing is added to the
/// steady-state path, which remains one `Relaxed` load from a private line.
#[repr(C)]
pub(crate) struct OnceSite {
    site:       &'static LogSite,
    fired:      AtomicBool,
    /// `OnceCounted` only; stays 0 and unread for plain `Once`.
    suppressed: AtomicU32,
    next:       AtomicPtr<OnceSite>,
}
static ONCE_SITES: AtomicPtr<OnceSite> = AtomicPtr::new(core::ptr::null_mut());
// SAFETY: insert-only, never removed, never freed — every node is a `'static`.
// A pusher publishes `site`/`fired` before the CAS that links `next`, so a
// reader observing a non-null pointer via `Acquire` observes a complete node.
// The list is walked only by the census, which tolerates a node appearing
// between two walks; a site absent from the list is a site that never fired,
// and that absence IS the datum.

// ────────────────────────── boyko_log/src/sink/*.rs ───────────────────────────

/// Sink KIND is fixed at boot; STATE and FILTER are runtime-mutable by a byte
/// store from any thread. Lifecycle ops (open/close/retarget) go through
/// SINK_REQ so that no syscall and no allocation ever runs on a caller's
/// thread (Decision 23, G13).
#[repr(C, align(64))]
struct SinkSlot {
    state:  AtomicU8,          // Off | Active | Paused | Closing | Faulted
    floor:  AtomicU8,          // minimum level this sink accepts
    kind:   SinkKind,          // Console | File | Binary | Callback | Crash
    filter: [AtomicU64; 4],    // 256-bit target mask; bit i == target i accepted
    _pad:   [u8; 26],
    // The handle / cursor / rotation state is owned by the SINK THREAD alone.
}
pub const MAX_SINKS: usize = 8;
static SINKS: [SinkSlot; MAX_SINKS];

#[repr(C, align(64))]
struct SinkReq { seq: AtomicU32, op: u8, slot: u8, path_len: u16, path: [u8; 256] }
static SINK_REQ: [SinkReq; 16];          // full ⇒ boyko-E0107, never silent
static SINK_REQ_HEAD: AtomicU32; static SINK_REQ_TAIL: AtomicU32;

// The three consumer-role scratch buffers. ALL THREE ARE `.bss` STATICS, not
// heap — v2 left `STAGE_BYTES`'s backing store unspecified, and "no `Vec`/`Box`
// in any SIGNATURE" is narrower than the claim a reader takes from it (F25).
// They are counted in Decision 3's budget matrix.
static STAGE:     UnsafeCell<[u8; STAGE_BYTES]>;       // 256 KiB — Algorithms C
static SITE_DICT: UnsafeCell<[SiteDictEntry; 4096]>;   // 64 KiB — binary sink only
static SINK_OUT:  UnsafeCell<[u8; 1 << 20]>;           // 1 MiB — binary write buffer
// SAFETY: all three are touched only by the thread currently holding the
// consumer role — the sink thread, or the crash-draining thread after the
// SINK_STATE CAS proved the sink thread cannot be inside a drain (Decision 24).

// ─────────────────── boyko_ecs seam: the ECS-visible surface ──────────────────

/// The durable, displayable log. Backed by the engine's own storage — a
/// `VmReservation`-backed byte column, NOT a `Box<[u8]>` heap side-store, which
/// is the shape Principle 0 was re-stated to forbid even inside a `Resource`
/// (M13). Fixed capacity, reserved at plugin build, never grows.
#[derive(Resource)]
pub struct LogRing {
    lines: VmColumn<LogLine>,  // engine storage
    arena: VmColumn<u8>,       // engine storage
    head: u32, len: u32, arena_cursor: u32,   // wrapping; Decision 21, test 20
    seq:  u64,                 // monotone record sequence — the reader's cursor
}

// ─── B1: `VmColumn` is !Send + !Sync; `Resource` requires both ───────────────
//
// Verified against the tree: `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70`
// states verbatim "NOT `Send`/`Sync` (the `NonNull` inside `VmReservation` and
// `base`): owners that cross threads carry their own exclusivity argument in
// their manual `unsafe impl Send/Sync` (SEND10 on `Archetype` …)", and
// `crates/boyko_ecs/src/ecs/core/resources/resource.rs:42` reads
// `pub trait Resource: 'static + Send + Sync + Sized`. v3 declared `LogRing`,
// `LogStats` and `LogCensus` "ordinary `Resource`s" two sections after
// Decision 12 deleted `LogHandle` for exactly this rule — the fold did not
// compile. `LogStats` is `Copy` POD and derives both; the other two need the
// impl below.
//
// COMPILE-TIME PIN — this is the F7 treatment applied to `Send`/`Sync` instead
// of to size. A future field that is not `Send`/`Sync` fails HERE, not in
// `LogPlugin::build`:
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LogRing>();
    assert_send_sync::<LogCensus>();
    assert_send_sync::<LogStats>();
};

// SAFETY (SEND10-shaped, for `LogRing` and `LogCensus`):
//   1. WHO MAY HOLD `&mut`: exactly one system, `log_drain_system`, which the
//      scheduler grants `ResMut<LogRing>` / `ResMut<LogCensus>`. The
//      scheduler's conflict analysis is what makes that exclusive; no other
//      system declares `ResMut` on either.
//   2. WHO MAY HOLD `&`: any system declaring `Res<..>` (a HUD, a console, a
//      telemetry reducer). The scheduler never runs a `Res` reader
//      concurrently with the `ResMut` writer, which is the same guarantee
//      every other `Resource` in this engine rests on.
//   3. WHO MAY NOT TOUCH THEM AT ALL: **the sink thread**. This is the clause
//      that makes the impl true rather than merely stated, and it is why B2
//      had to be answered first: the sink writes `ECS_HANDOFF` (a `.bss` byte
//      ring, Decision 26) and never names `LogRing` or `LogCensus`. If the
//      sink wrote the columns directly, clauses 1-2 would be false and the
//      only repair would be a lock — which Invariant 1 forbids.
//   4. WHY THE UNDERLYING COLUMNS TOLERATE IT — quoting `vm_column.rs:73-77`'s
//      own invariant list: `base` is write-once (set at lazy materialization
//      inside the `&mut self`-only `grow_to`, stable thereafter); every
//      mutation requires `&mut self`; cross-thread `&self` reads touch only
//      committed plain-old-data below `len` with no interior mutability.
//      `LogLine`, `u8` and `TargetStat` are all POD, so clause 4 holds for
//      every element type used here.
//   5. MATERIALIZATION IS NOT LAZY IN PRACTICE: `LogPlugin::build` calls
//      `grow_to` to the configured capacity BEFORE the schedule ever runs, so
//      the write-once `base` store happens once, single-threaded, at plugin
//      build. No `&self` reader can observe a partially materialized column.
unsafe impl Send for LogRing {}   unsafe impl Sync for LogRing {}
unsafe impl Send for LogCensus {} unsafe impl Sync for LogCensus {}

/// EXACTLY 16 BYTES, `Copy`, and pinned by a const assert *(fixes F7)*.
/// `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` panics in
/// `VmColumn::<T>::new` unless `COMMIT_GRANULE % size_of::<T>() == 0`, and
/// `COMMIT_GRANULE == 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`).
/// v2's layout was 12 bytes (10 payload + align-4 tail): `65536 % 12 == 4`, so
/// `LogPlugin::build` would have PANICKED at construction and rung L5 could not
/// have landed green. `repr(C, packed)` does not save it either (`65536 % 10
/// == 6`). The fix is a size that divides the granule, not an attribute — and
/// the const assert below turns "someone adds a field" from a plugin-build
/// panic into a compile error. `VmColumn<T>` also requires `T: Copy`, hence the
/// derive; it is `pub(crate)` to `boyko_ecs` (`vm_column.rs:80`), which is fine
/// because `LogRing` lives inside `boyko_ecs` and the field is private.
#[repr(C)] #[derive(Clone, Copy)]
pub struct LogLine {
    start:  u32,   // offset into `arena`
    seq_lo: u32,   // low half of the record's sequence number
    len:    u16,   // bytes of formatted text
    code:   u16,   // 0 when the level carries none
    level:  u8,
    target: u8,    // MAX_TARGETS == 256 fits a u8 exactly
    flags:  u8,    // STR_TRUNCATED | SUPPRESSED_FOLLOWS | SAMPLED_CONTEXT
    _pad:   u8,
}
const _: () = assert!(core::mem::size_of::<LogLine>() == 16);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<LogLine>() == 0,
    "LogLine must divide COMMIT_GRANULE or VmColumn::new panics (F7)");
// `VmColumn<u8>` for the arena is trivially fine: 65536 % 1 == 0.

/// Monotonic counters; zero per-frame allocation. Mirrors the shape of
/// `crates/boyko_app/src/window_info.rs:34`'s `HostFrameStats` — counters only,
/// no timing fields, because that struct HAS no timing fields (checked).
#[derive(Resource, Clone, Copy, Default)]
pub struct LogStats {
    pub emitted: u64, pub dropped: u64, pub dropped_bytes: u64,
    pub suppressed: u64, pub unlaned_dropped: u64, pub sampled_out: u64,
    /// `LossClass::Sink` on `ECS_HANDOFF` — formatted lines that reached the
    /// byte sinks but not the in-frame view (Decision 26, `W0117`).
    pub handoff_lost: u64,
    /// Emissions that ran with no rate state because the 512-slot code-index
    /// space was exhausted (M3, `E0115`). Never an aliased slot.
    pub codes_unindexed: u64,
    pub lanes_claimed: u32, pub lanes_retired: u32,
    /// Spares held for the process by threads that never called
    /// `boyko_diag::release_lane()`; bounded at 14 (S3).
    pub lanes_leaked: u32,
}

/// Per-target counts for an in-game overlay / support HUD / telemetry payload.
/// `lossy` is the one bit a UI must read before showing a count as a total
/// (Decision 17).
#[derive(Resource)]
pub struct LogCensus {
    per_target: VmColumn<TargetStat>,   // MAX_TARGETS rows, engine storage
    /// `boyko_diag::SessionId` — ONE mint shared with the profiler's artifact
    /// header (S11), so an uploaded log and an uploaded artifact identify the
    /// same session. v3 minted its own.
    pub session: boyko_diag::SessionId,
    pub lossy: bool,
    pub control_epoch: u32,             // CONTROL_EPOCH_CTR at the last drain (S11 rename)
}

/// The SINGLE game-facing diagnostics surface (S8). A game asks one question
/// about completeness rather than two that can disagree — and the `lossy` bit
/// here is the OR of both subsystems', so a UI cannot render one as a total
/// while the other was dropping.
#[derive(Resource)]
pub struct DiagCensus { pub log: LogCensus, pub prof: ProfCensus, pub lossy: bool }
#[repr(C)] #[derive(Clone, Copy)]
pub struct TargetStat { pub delivered: u64, pub dropped: u64,
                        pub sampled_out: u64, pub sync_routed: u64 }
const _: () = assert!(core::mem::size_of::<TargetStat>() == 32);
const _: () = assert!(COMMIT_GRANULE % core::mem::size_of::<TargetStat>() == 0);
// NOTE: there is no `LogFilter`. `CONTROL` is the single owner (Decision 14).
```

---

## Public API

```rust
// ── declaration ───────────────────────────────────────────────────────────────
/// ENGINE targets: one central table, IDs 0..=95, uniqueness proven at COMPILE
/// time by a strictly-increasing const assert (Decision 15).
targets! {
    (0,  Ecs,        "ecs",           Level::Info),
    (1,  Schedule,   "schedule",      Level::Info),
    (2,  Threadpool, "threadpool",    Level::Warn),
    // …
}
/// DOWNSTREAM SOURCE targets: IDs 96..=223, boot-checked (`boyko-E0104`).
#[macro_export] macro_rules! define_target { ($vis:vis $Ty:ident, name=$n:literal, id=$id:literal, ceiling=$c:expr) => {...} }

/// DYNAMIC targets: IDs 224..=255, minted from data/mod/script names.
/// COLD, setup-time, idempotent by name. `None` => band exhausted (`boyko-E0106`).
pub fn register_dynamic_target(name: &str, initial: TargetControl) -> Option<TargetId>;
pub fn find_target(name: &str) -> Option<TargetId>;         // #[cold], linear scan
pub fn targets() -> TargetIter<'static>;                    // #[cold], settings screens

// ── emission: static targets (gates a+b const-folded) ─────────────────────────
#[macro_export] macro_rules! error { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! warn  { ($T:ty, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! info  { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! debug { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! trace { ($T:ty,             $fmt:literal $(, $a:expr)*) => {...} }

/// Named-field form; names live in the cold `LogSite`, so this costs the same
/// as the positional form on every hot path (Decision 19b).
/// `info_kv!(Combat, "hit", dmg = d, target = t)`
#[macro_export] macro_rules! info_kv  { ... }   // + debug_kv! trace_kv! warn_kv! error_kv!

// ── emission: dynamic targets (gate (a) unavailable — Decision 18) ────────────
#[macro_export] macro_rules! dyn_info  { ($id:expr, $fmt:literal $(, $a:expr)*) => {...} }
#[macro_export] macro_rules! dyn_warn  { ($id:expr, $code:expr, $fmt:literal $(, $a:expr)*) => {...} }
// + dyn_debug! dyn_trace! dyn_error!

/// Renders a `Display` into a stack buffer owned by the ARGUMENT EXPRESSION.
/// Expansion form pinned in Decision 13 — the naive block form does not
/// borrow-check (F22).
#[macro_export] macro_rules! dsp { ($e:expr) => {...}; ($e:expr, $n:literal) => {...} }

// `report!` IS DELETED (S1). The measurement channel belongs to the profiler
// end to end; its durable output is an artifact, never stdout. NOTHING in this
// crate writes stdout. `OUT_LOCK` survives — see below for its callers.

/// SYNCHRONOUS, ordered, never dropped, under OUT_LOCK's BOUNDED acquire
/// (Decision 9c). Fans out to EVERY configured synchronous destination:
/// `stderr()`'s own handle when a ConsoleSink exists, AND the boot-opened
/// crash file when a CrashSink exists (B9 — v3 wrote stderr unconditionally,
/// which made three "durable" promises inert in `shipping`/`shipping-min`).
/// Callers: the lane-exhaustion fallback for Warn/Error (Decision 5),
/// sync-routed targets (Decision 20), the pre-boot / post-shutdown Warn/Error
/// fallback (Decision 12), the panic hook, and flush's timeout path.
/// NOT the Vulkan messenger — that site is not touched at all (Decision 9b).
/// Not for ordinary diagnostics.
pub fn write_oracle_line(prefix: &str, body: &[u8]);

// ── values ────────────────────────────────────────────────────────────────────
pub trait LogValue: private::Sealed {
    const MAX_ENCODED_LEN: usize;
    fn encoded_len(&self) -> usize;
    unsafe fn encode(&self, dst: *mut u8) -> usize;
}
// impls: {i,u}{8,16,32,64,128}, f32, f64, bool, char, &'static str, &str.

/// Game-extensible POD values. Blanket-bridged into `LogValue`, so sealing is
/// preserved and Decision 13's structural property is untouched: the encoder
/// is generated from `LogValue`, `fmt_pod` runs on the sink (D19b, test 24).
/// `POD_LEN` is the SUM OF FIELD ENCODED LENGTHS, not `size_of::<Self>()`, so
/// no padding byte ever reaches a sink (B10).
pub unsafe trait LogPod: Copy + Send + Sync + 'static {
    const POD_LEN: usize;
    /// # Safety
    /// `dst` valid for `POD_LEN` writes; the impl writes exactly that many
    /// INITIALISED bytes.
    unsafe fn encode_pod(&self, dst: *mut u8);
    fn fmt_pod(bytes: &[u8], f: &mut LogFormatter);
}
// boyko_macros: #[derive(LogPod)] — requires #[repr(C)] and all-LogValue
// fields, REJECTS dynamic-length fields (`&str`), and generates `encode_pod`
// field-by-field plus `const _: () = assert!(POD_LEN == Σ field lengths)`.

// ── codes ─────────────────────────────────────────────────────────────────────
/// Engine registry (single invocation, `prefix = "boyko"`), and the SAME macro
/// exported for downstream tables (Decision 19).
#[macro_export] macro_rules! codes { (prefix = $p:literal, doc_root = $d:literal, $($row:tt)*) => {...} }
/// Generates the EIGHT registry checks over a caller-supplied root+prefix.
/// The engine's own checks prove nothing about a downstream crate.
#[macro_export] macro_rules! codes_tidy { (root = $r:literal, prefix = $p:literal) => {...} }

// ── runtime control (any thread, no lock, no restart — Decision 23) ───────────
pub fn target_control(id: TargetId) -> TargetControl;
pub fn set_target_control(id: TargetId, ctl: TargetControl);   // CAS, preserves siblings
pub fn set_target_level(id: TargetId, lvl: Level);             // CAS, preserves shift+sync
pub fn control_epoch() -> u32;                                 // O(1) repaint signal
pub fn apply_control_spec(spec: &str) -> Result<u32, ControlSpecError>; // "net=debug/6!"

pub fn sink_state(slot: u8) -> SinkState;
pub fn set_sink_state(slot: u8, s: SinkState);                 // byte store, any thread
pub fn set_sink_filter(slot: u8, targets: &[u64; 4], floor: Level);
pub fn request_open_file(slot: u8, path: &str) -> Result<(), SinkReqError>;  // E0107 if full
pub fn request_close(slot: u8) -> Result<(), SinkReqError>;

// ── lifecycle ─────────────────────────────────────────────────────────────────
pub struct LogConfig {
    pub sink_mode: SinkMode,            // Thread (default) | Manual | Scheduled (B8)
    pub console:   Option<ConsoleSink>, // stderr's own handle; colour/level floor (S7)
    pub file:      Option<FileSink>,    // path, rotation
    pub binary:    Option<BinarySink>,  // path, rotation, flush_interval_ms
    pub crash:     Option<CrashSink>,   // path; OPENED AT BOOT (Decision 24)
    pub callback:  Option<CallbackSink>,// extern "C" fn(&FormattedRecord, *mut ()) + ctx
    pub ecs_ring:  bool,                // enables ECS_HANDOFF + LogRing (D26)
    /// `write_oracle_line` also `sync_data()`s the crash handle. ~0.1-10 ms per
    /// record; OFF by default, because a sync bit that also fsync'd would
    /// serialise the frame on the disk rather than on the format (B9).
    pub sync_durable: bool,
    pub census:    CensusPolicy,        // OnFlush (dev) | OnShutdown | Interval(secs)
    pub control_source: ControlSource,  // None | Env | File(&'static str)
    pub default_controls: [TargetControl; MAX_TARGETS],
}
/// RENAMED from "the `LogConfig` profile" (S9) and it has NO ceiling column:
/// `GLOBAL_CEILING` and `LANE_COUNT` are compile-time consts from
/// `BOYKO_PROFILE`, which a runtime preset cannot deliver. The header prints
/// `build_profile` / `runtime_preset` / `ceiling` as three independent facts.
pub enum LogRuntimePreset { Dev, Editor, Shipping, ShippingMin, Off }
impl LogRuntimePreset { pub fn config(self) -> LogConfig; }

pub struct Rotation { pub max_bytes: u64, pub keep: u8 }   // Rotation::NONE = v2 behaviour
pub enum LogBootError { AlreadyBooted, TargetIdCollision { id: u16, a: &'static str, b: &'static str }, SinkOpen(std::io::Error) }
pub enum FlushResult { Flushed, NoConsumer, TimedOut }
/// `Busy` is why `drain()` returns something: a second manual caller is a USER
/// error, not a bug in this crate, so it is refused rather than asserted (B5).
pub enum DrainResult { Drained { records: u32 }, Busy, NoLanes }

pub fn boot(cfg: LogConfig) -> Result<(), LogBootError>;  // no handle (Decision 12)
pub fn shutdown();                                        // idempotent
pub fn flush() -> FlushResult;                            // never waits on a dead consumer
pub fn drain() -> DrainResult;                            // claims DRAIN_OWNER, or Busy
/// Registers an `extern "C" fn()` called by `flush()`, by `shutdown()` and by
/// the panic hook BEFORE the crash drain (S5). Eight slots; a ninth is
/// `boyko-E0118`. A registrant must not allocate, must not lock, must not
/// touch the `World`, and must do at most one `write_all`.
pub fn register_pre_flush(f: extern "C" fn()) -> Result<(), PreFlushFull>;
pub fn session_id() -> boyko_diag::SessionId;             // ONE mint (S11)
pub fn explain(code: u16) -> Option<&'static DiagInfo>;
pub fn census() -> CensusIter<'static>;                   // Measured / Unproven per target
pub fn name_current_thread(name: &'static str);           // cosmetic, cold, once

// ── ECS seam (boyko_ecs) ──────────────────────────────────────────────────────
pub struct LogPlugin { pub config: LogConfig }
impl Plugin for LogPlugin { fn build(&self, app: &mut App); }
// inserts LogRing / LogStats / LogCensus; adds `log_drain_system` to `Last`
// (ECS ring feed + TARGET_STATS snapshot + one EPOCH record per frame — the
// sink thread owns the byte sinks). Registers `shutdown` on teardown.

impl LogRing {
    /// Records delivered since `cursor`, oldest first. `cursor` is a monotone
    /// sequence number; a gap means the ring wrapped and `LogRingIter::skipped`
    /// says by how much — a console cannot silently miss lines (Decision 26).
    pub fn since(&self, cursor: u64, filter: &RingFilter) -> LogRingIter<'_>;
    pub fn cursor(&self) -> u64;
}
pub struct RingFilter { pub targets: [u64; 4], pub min_level: Level }
```

No `Vec`, `Box<dyn>`, `HashMap` or internal type appears in any signature. The callback seam is an `extern "C" fn` + ctx, so it crosses a dylib boundary with no vtable and no allocation.

---

## Algorithms for critical paths

### A. `emit` — the producer hot path

```
1. GATE (inlined into the caller)
   a. T::STATIC_CEILING       >= LVL           — const, folded  [ABSENT for dyn_* — D18]
   b. $crate::GLOBAL_CEILING  >= LVL           — const, folded
   c. ctl = CONTROL[T::ID].load(Relaxed);  (ctl & 0x07) >= LVL  — 1 B L1 load + and + cmp
   Fail ⇒ nothing. Arguments NEVER evaluated (&& short-circuit).
   [Arguments, incl. any `dsp!`, are evaluated HERE, before step 2.]

1b. SYNC ROUTE (bit 7 of ctl, predicted not-taken)
   ctl & 0x80 ⇒ format on this thread, write_oracle_line() (BOUNDED — D9c),
   TARGET_STATS.sync_routed += 1, return.  ~200+ ns, per-target opt-in (D20).

2. RATE (Warn/Error only) — code-indexed and site-indexed, NOT lane-indexed,
   so it stays ahead of step 3 and a suppressed record never pays a lane claim
   Once/OnceCounted && FIRED.load(Relaxed) ⇒ [OnceCounted: suppressed RMW] return
                                             // per-SITE static, private line (F11)
   EveryN(n)  ⇒ (count.fetch_add(1) & (n-1)) != 0 ⇒ return   // n is pow2 (X3)
   MinInterval⇒ policy RMW
   Every      ⇒ skip
   Downstream code: idx = idx_cell.load(Relaxed); UNASSIGNED ⇒ #[cold] mint (D19)
                    idx == CODE_IDX_EXHAUSTED ⇒ Every semantics, no rate state,
                    codes_unindexed += 1 (M3)

3. LANE  = `boyko_diag::lane()` (S3 — a Cell<u16> TLS read, no Drop, no claim
   scan in this crate). UNCLAIMED ⇒ #[cold] `boyko_diag::claim_lane()`.
   Still none ⇒ Warn/Error: write_oracle_line() (synchronous fallback with the
   durable fan-out, M26 + B9); else LossClass::Unclaimed += 1; return.
   // MOVED AHEAD OF SAMPLING (B4). v3 resolved the lane at step 4 while step 3
   // already indexed `SAMPLE_CTR[lane][target]` — the row index was read
   // before it existed, and an unlaned thread (E6) had no row at all. Only
   // SAMPLE is lane-indexed, so this is the minimal reordering that fixes it:
   // RATE keeps its position, and a rate-suppressed record on a fresh thread
   // still costs no lane claim.

4. SAMPLE (k = (ctl >> 3) & 0x0F; predicted not-taken when k == 0)
   c = SAMPLE_CTR[lane][target].get().wrapping_add(1)
   SAMPLE_CTR[lane][target].set(c)                    // plain load/store, NO RMW
   (c & ((1<<k)-1)) != 0 ⇒ sampled_out += 1; return
   // NOTE, and it is a user-visible property rather than an implementation
   // detail: ARGUMENTS WERE ALREADY EVALUATED AT STEP 1. Sampling suppresses
   // DELIVERY, never argument evaluation. A side-effecting argument runs on
   // every occurrence regardless of `k`. G10e asserts both numbers in one leg
   // — 1000 evaluations, 500 delivered — which is the distinction v3's G4 leg
   // (d) collapsed by asserting 500 for a quantity that is 1000 (B4).

5. SIZE   need = HEADER_BYTES + args.encoded_len()          // runtime (B2)
   need > MAX_RECORD_BYTES ⇒ drop + TOO_LARGE count; return  // N29, release-live

6. SPACE + WRAP  (records never straddle — B3; arithmetic corrected — F6)
   w    = write.load(Relaxed);  off = w & MASK
   tail = LANE_BYTES - off
   // Rule shared VERBATIM by producer and consumer:
   if tail < HEADER_BYTES { pad = tail }                     // implicit wrap
   else if tail < need    { pad = tail }                     // explicit PAD record
   else                   { pad = 0 }

   // ── admission control. NO unsigned subtraction here can go negative. ──
   //   CAPACITY = LANE_BYTES - 1        (one slot reserved: full vs empty)
   //   INVARIANT used <= CAPACITY, inductive over the producer's own
   //   admissions: read_cached <= read <= w always, and the producer only
   //   publishes w + pad + need after proving pad + need <= avail.
   used   = w.wrapping_sub(read_cached)
   debug_assert!(used <= CAPACITY);
   avail  = CAPACITY - used                                  // cannot underflow
   budget = if level == Error { avail } else { avail.saturating_sub(ERROR_RESERVE) }
   //       ^ SATURATING is the fix. v2 computed `LANE_BYTES - ERROR_RESERVE
   //         - used`, which underflowed to ~4.29e9 in exactly the state the
   //         reserve exists to create, and licensed an overrun of live bytes.
   if budget < pad + need {
       read_cached = read.load(Acquire); recompute used/avail/budget;
       if still short {
           if dropped.load(Relaxed) != u32::MAX {             // saturation (D5, D21)
               dropped.fetch_add(1, Relaxed);
               dropped_bytes saturating-add need;
           }
           return
       }
   }
   if pad >= HEADER_BYTES { write PAD header (site = null, len = pad) at off }
   w = w.wrapping_add(pad); off = w & MASK   // now off == 0 or tail >= need

7. WRITE   write_unaligned(off, RecordHeader{ site, tsc: boyko_diag::ticks(),
                                              len: need, flags,
                                              clock_epoch_lo: epoch as u8 })
           args.encode(off + HEADER_BYTES)
8. PUBLISH write.store(w.wrapping_add(need), Release)
```

- **Complexity** O(1); O(len) memcpy for inline `&str`.
- **Cache — the isolated figure and the joint one, together, because quoting one without the other is how a budget gets believed.** Strictly sequential streaming writes into the ring tail. **In isolation** the working set is the `CONTROL` line, the producer line, the lane's `SAMPLE_CTR` row segment (one line) and 1-2 ring-tail lines — **≤ 4 lines**, unchanged from v2, because the sampling row is the only addition and it is one line and producer-private. A `Once` site's `FIRED`/`OnceSite` static is a fifth line only on the `Warn`/`Error` path, which is not the budgeted path. **Jointly with the profiler armed the same producer also touches `ARM_MASK`, the `ZoneLane` control line and the sample tail, for 7-8 distinct lines** (seam record §Joint cost) — and the shared TLS slot means 1, not 2, of those lines is the lane id. `LOG_LANES`, `CONTROL` and `SAMPLE_CTR` have compile-time-known addresses — no pointer chase.
- **Branching** 3 (or 2, dynamic) predicted-not-taken gates + sync + rate + sample + wrap + space. `budget` is a `saturating_sub`, i.e. `sub` + `cmov` — still branchless. The sync and sample branches are not-taken in every default configuration, so I-cache pressure is one extra `cmp/jcc` pair each.
- **Inlining** steps 1-3 `#[inline]` (must fold). Steps 4-8 in `#[inline(never)] fn emit_impl<A: LogArgs>` — monomorphised per argument-tuple type. Blanket `#[inline(always)]` would replicate ~60 instructions at every site and bloat L1i, which principle 7 forbids on measurement grounds.
- **SIMD** none wanted: the payload is ≤ 2 KiB and moves by `copy_nonoverlapping`, which already lowers to the best available move sequence. There is no vectorisable reduction anywhere in `boyko_log`.

### B. Lane resolution / retire — the claim scan lives in `boyko_diag` now *(S3)*

```
RESOLVE (hot, one TLS read):
  id = boyko_diag::lane()                       // Cell<u16>, NO Drop, no allocation
  if id == LANE_UNCLAIMED { id = boyko_diag::claim_lane()? }   // #[cold], once/thread
  lane = LOG_LANES.get(id as usize)?            // None in the `off` build (len 0, F21)
  first touch of this lane by this thread ⇒
      seed SAMPLE_CTR[id][t] = (id * 0x9E37) as u16 for all t  // phase break (D20)

RETIRE (producer thread, explicitly, after its last write):
  boyko_diag::release_lane()                    // marks the SUBSTRATE slot RETIRING
  // NOT a `Drop` guard. v3 installed a `thread_local!` with a destructor; that
  // was the mechanism the profiling plan refused AND the sole source of this
  // plan's "<= 1 allocation on first emit" row. Deleting it takes that row to 0
  // (S3). A thread that simply exits without calling `release_lane()` leaks its
  // spare for the process — bounded at 14 x LANE_BYTES, counted as
  // `lanes_leaked`, printed in the census. That is the price of not having a
  // destructor, and it is paid in a bounded, counted, printed quantity rather
  // than in an allocation on every thread's first emit.

RECLAIM (consumer, per drain, after staging):
  if boyko_diag::lane_state(i) == RETIRING && read == write {
      boyko_diag::reclaim(i)                    // ONE registry owns the transition
  }
```

**What this crate no longer contains**: `MAX_LANES`, the `hash(thread_id)` spread, the `load`-then-CAS scan over `LANES`, the `owner` field, `MY_LANE`, and the TLS guard type. Five deletions, one dependency.

### C. Drain — staged copy BEFORE the free *(fixes B1)*

```
0. CLAIM THE CONSUMER ROLE (B5). Every consumer does this, identically:
   if DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire).is_err() {
       return Busy    // sink: re-park. Manual: DrainResult::Busy.
   }                  // Scheduled: skip this frame. Crash: do not displace.
   // Released by an RAII guard, so an unwind inside the drain does not strand
   // the role — the same shape as OutGuard (D9c).

STAGE_BYTES = 256 KiB (a `.bss` static, consumer-role-owned, reused every drain — F25)

drop_tally = {lanes: 0, records: 0, bytes: 0, by_class: [0; LOSS_CLASSES]}
for each lane:
  w = write.load(Acquire); r = read
  while r != w and staging has room:
      off = r & MASK
      if LANE_BYTES - off < HEADER_BYTES { r += LANE_BYTES - off; continue }  // shared wrap rule
      hdr = read_unaligned::<RecordHeader>(buf + off)     // TYPED copy: provenance-preserving
      if hdr.site.is_null() { r += hdr.len; continue }    // PAD
      copy_nonoverlapping(buf + off + HEADER_BYTES, stage + s + HEADER_BYTES, hdr.len - HEADER_BYTES)
      write::<RecordHeader>(stage + s, hdr)               // TYPED write: provenance-preserving
      push (hdr.tsc, lane, s) into the preallocated index scratch
      s += hdr.len; r = r.wrapping_add(hdr.len)
  read.store(r, Release)          // ← ONLY NOW is the ring space published free
  fold this lane's LossCell[..] / sampled_out into drop_tally, TARGET_STATS and
      LogStats; clear each with fetch_sub(observed) — NEVER store(0), because a
      producer increment between the load and the clear would be lost (S8)
  reclaim via boyko_diag if RETIRING && r == w
if drop_tally.records > 0 { synthesise ONE boyko-W0102 for the whole drain,
      carrying lanes_affected / records / bytes / the LossClass breakdown }
      // one per DRAIN, not one per lane: 125/s instead of ~16 000/s (F24)
sort_unstable_by_key(tsc)         // over preallocated 16 B triples; no allocation
for each staged record, for each ACTIVE sink whose filter accepts (target, level):
    text sinks : (*site.decode)(stage + s + HEADER_BYTES, len, &mut fmt)  // reads STAGING
                 // the sink renders `clock_epoch_lo` beside the timestamp (S4)
    binary sink: site_id = SITE_DICT lookup (#[cold] miss ⇒ dictionary record;
                 FULL table ⇒ W0116 once + an inline site record — M2)
                 append {site_id, tsc_delta, len, flags, clock_epoch_lo, payload}
                 // NO formatting (D22); anchor re-emitted at 1 s or u32 overflow
one write_all per byte sink per fill
if cfg.ecs_ring {
    push each formatted line into ECS_HANDOFF (D26) — the SAME wrap rule as a
    LogLane; on refusal, LossClass::Sink += 1 and ONE boyko-W0117 per drain.
    // The sink NEVER touches LogRing or LogCensus. That is what makes B1's
    // Send/Sync argument true rather than merely written.
}
release DRAIN_OWNER (RAII)
```

**Fan-out is inside ONE drain.** Every sink reads the same staging arena; there is never a second consumer of a lane. That is what makes "text + binary + crash simultaneously" cost one pass, and it is why §Refused rejects a second sink thread.

**Why the order changed.** v1 advanced `read` at step 3 and decoded at step 6 from an `offset` **into the ring** — bytes the producer was licensed to overwrite in between. The sink would then read a torn header and call `decode` through 8 arbitrary bytes reinterpreted as a function pointer. v1's tests could not see it: both the ordering test and the overflow test drive a quiesced producer. The staged copy makes the window structurally absent, and adds a bound: a drain never stages more than `STAGE_BYTES`, so a hot lane is drained across several passes rather than in one unbounded burst.

**Provenance.** The header — the only field carrying a pointer — is moved by a **typed** `read_unaligned`/`write` pair, never by a byte memcpy, so `site`'s provenance round-trips by construction rather than by relying on per-byte provenance tracking. Payloads are pointer-free POD and move by `copy_nonoverlapping`. Gated by Miri under Tree Borrows (test 14).

- **Complexity** O(R log R) per drain for R records, entirely off the frame thread.
- **Stated limitation** cross-lane ordering is *approximate*: a record written after lane A's snapshot may carry an earlier `tsc` than one already staged from lane B. Inherent to any non-blocking merge (Quill has the same property) and printed in the sink's header line.

### D. `flush`

```
0. run every registered PRE_FLUSH callback (S5) — before anything else, so a
   registrant's last window reaches disk even if step 1 short-circuits
1. match SINK_STATE.load(Acquire) {
       NotBooted | Manual | Scheduled | Exited => return FlushResult::NoConsumer,
       Running | Exiting => {}
   }
   // `Scheduled` joins the short-circuit set: its consumer is the schedule,
   // which `flush()` cannot unpark and must not impersonate (B8 + B5).
2. seq = FLUSH_SEQ.fetch_add(1, AcqRel) + 1        // RENAMED from FLUSH_REQ's
                                                   // "epoch" — S11, three meanings
3. unpark(sink)
4. spin_backoff until FLUSH_ACK.load(Acquire) >= seq, deadline = now + 2 s
5. on timeout: write_oracle_line("boyko-E0105: log flush timed out"); TimedOut
   // write_oracle_line is BOUNDED (≤ 50 ms, then steal) — Decision 9c. v2's
   // bounded wait terminated in an UNBOUNDED one (F8).
```
Step 1 is what keeps `#[should_panic]` tests in an unbooted binary at zero cost. Step 5 is non-negotiable: the profiling audit's central finding is that an unbounded blocking wait converts an instrumentation gap into an unkillable hang, and that this repository has no kill-after-timeout pattern to borrow (`vb_bench_totality_gate.rs:48-49`). This design does not add a second one.

### E. Crash drain *(Decision 24)*

```
panic hook (chained ahead of the existing hook):
1.  write_oracle_line(panic message)                      // synchronous, bounded,
                                                          // durable fan-out (B9)
1.5 run every registered PRE_FLUSH callback (S5)          // BEFORE the crash drain;
                                                          // no alloc, no lock, no World
2.  match flush() { Flushed => return, _ => {} }
3.  if DRAIN_OWNER.compare_exchange(0, my_token, AcqRel, Acquire).is_ok() {
        // The consumer role is now EXCLUSIVE BY POSSESSION, not by inference
        // from a state that merely correlates with it (B5). v3 CAS'd
        // SINK_STATE out of {Exited, NotBooted, Manual} — and `Manual` means
        // an arbitrary thread may be INSIDE drain() right now, so v3 could
        // start a second consumer over these lanes in the one profile
        // (`shipping-min`) that shipped it.
        run Algorithms C over every lane, text sink = CRASH sink only
        write_oracle_line("boyko-E0109: crash drain took the consumer role")
        release DRAIN_OWNER; SINK_STATE.store(Exited, Release); return
    }
4.  // Someone else holds the role — a live sink thread, a manual drain(), or a
    // scheduled drain. DO NOT displace it: two consumers on one lane is what
    // the LogLane SAFETY block forbids. Whatever that consumer has already
    // staged, IT will write; the rest is lost and counted. flush() already
    // timed out and said so (E0105).
    return
```
- **Termination**: step 3 is a single CAS; step 4 returns. No wait is added.
- **What it cannot do**: survive `abort()`, `SIGSEGV`, or a guard-page stack overflow — the hook does not run. Written down in E22 and in G14's "cannot claim" column rather than mitigated by a heuristic.

---

## Multithreading model

| Datum | Sharing | Ordering | Why |
|---|---|---|---|
| `LogLane::buf` | SPSC | none (guarded by `write`) | payload published by the cursor's Release |
| `LogLane::write` | P→C | `Release` / `Acquire` | the happens-before edge for the payload |
| `LogLane::read` | C→P | `Release` (after staging) / `Acquire` | frees space only once bytes are copied out (B1) |
| `read_cached` / `write_cached` | private `Cell`, single-role | none | the half that actually buys throughput; SAFETY clauses 1e/1f cover them (F23) |
| **lane identity** | owned by `boyko_diag` | `Cell<u16>` TLS read (no atomic); `claim_lane` is load-then-CAS `Acquire`, `release_lane` `Release` | **not this crate's datum any more** (S3). Contended once per thread lifetime, on a `#[cold]` path, over 14 spares |
| `LossCell[class]` (per lane) | **single writer** = lane owner; consumer folds | plain `u64` load/store by the owner; `fetch_sub(observed)` on the `AtomicU64` total | own cache line; no lock prefix on the producer at all (S8). `fetch_sub` never loses a concurrent add — a `store(0)` would |
| `sampled_out` | P adds, C folds | `Relaxed` | not a `LossClass`; kept separate so `emitted == drained + dropped + sampled_out` stays exact |
| **`DRAIN_OWNER`** | MPMC, once per drain pass | CAS `AcqRel` / `Acquire` on failure; RAII `Release` | **the object clause 2 of the `LogLane` SAFETY block is about** — so it is the object that is CAS'd (B5). Uncontended in every normal configuration; contended only when a panic races a drain |
| `CONTROL[i]` | MP-read, rare CAS write | `Relaxed` read, `AcqRel` CAS | a stale ceiling for one record is documented as acceptable; the CAS preserves sibling bit-fields (D14) |
| `CONTROL_EPOCH_CTR` | 1W-ish / MR | `Release` add / `Acquire` load | derived and monotone; carries no state, so it cannot diverge from `CONTROL` |
| **`ECS_HANDOFF.write` / `.read`** | SPSC: producer = `DRAIN_OWNER` holder, consumer = `log_drain_system` | `Release` / `Acquire`, both cursors | identical to `LogLane`'s pair — one protocol, two instances (D26, B2). Overflow is a counted refusal (`LossClass::Sink`, `W0117`), never a silent drop |
| **`ONCE_SITES` head / `next`** | MP push, insert-only | CAS `AcqRel` push, `Acquire` walk | one push per site per **process**, on the `#[cold]` single-fire branch; the census walks it (M1). Nothing is ever removed or freed |
| **`PRE_FLUSH[n]`** | MP claim, MR call | CAS `AcqRel` claim, `Acquire` load | eight slots, claimed at boot; a ninth is `E0118` (S5) |
| `SAMPLE_CTR[lane][t]` | **single writer** = lane owner | `Relaxed` load + store, **never an RMW** | the row index IS the lane index ⇒ SAFETY clause 1 applies verbatim; no sharing, no lock prefix |
| per-site `FIRED` (`Once`) | MP | `Relaxed` load; one lifetime `swap` | steady state is a pure load from a **site-private** line (M11, F11) |
| `RATE[idx]` (`EveryN`/`MinInterval` only) | MP | `Relaxed` RMW | opt-in; cost documented at the declaration site |
| `CodeIdx::Dynamic` cell | MP | CAS `UNASSIGNED→RESERVED`, `fetch_add`, `store(Release)`; readers `Acquire` | reserve-then-publish: dense, no leaked indices, one CAS per code per process (D19) |
| `DYN_NAMES[i].hash` | MP insert-only | bytes+len stored, then `hash.store(Release)`; readers `Acquire` | a reader seeing a hash sees a complete name |
| `TARGET_STATS[i]` | C writes, MR reads | `Relaxed` | one writer (the consumer-role holder); readers tolerate one-drain staleness, which the census states |
| `SINKS[n].state` / `.filter` / `.floor` | MW stores, C reads | `Relaxed` | policy only; acting on a one-drain-stale filter is a documented property (G13), not a race |
| `SINK_REQ` | MP producers, 1 consumer | `AcqRel` seq; writes under `OUT_LOCK` | cold, human-initiated, bounded at 16; full ⇒ `E0107` |
| `FLUSH_SEQ` / `FLUSH_ACK` / `SINK_STATE` / `SINK_EXITED` | 2-way | `AcqRel` / `Acquire` | completion must be observed, not guessed. **`SINK_STATE` no longer carries the exclusivity proof** — `DRAIN_OWNER` does (D24, B5) — so it has one job, lifecycle, and `CrashDraining` is deleted from it |
| `OUT_OWNER` (`OUT_LOCK`) | MP | CAS `Acquire` / `Release`; **bounded acquire, RAII release, re-entrancy detected** | callers: `write_oracle_line` (console sink, sync-routed targets, exhaustion fallback, pre-boot/post-shutdown severe records, panic message, flush timeout) and `SINK_REQ` writes. `report!` is gone (S1); the lock is not. The Vulkan messenger is **not** a caller (D9b) |
| Sink array kinds, `LogConfig` | boot-published | one `Release` at boot | kinds never mutate after boot; only state/filter/floor do |

**Data-race freedom.** No lane has two producer *threads*: `boyko_diag::lane()` returns an index unique per live thread, dense-by-construction for pool workers and CAS-claimed for everyone else (S3). No lane has two producers *re-entrantly* on one thread (no user code runs inside the open window — Decisions 13 and 19b, `debug_assert`ed and asserted by test 24; and after B10 the `LogPod` encoder is generated from `LogValue`, so it is ours). **No lane ever has two consumers**: all four consumers — sink thread, `drain()`, `log_drain_system` under `Scheduled`, and the crash drainer — acquire the **same CAS'd `DRAIN_OWNER` token** (Decision 24). v3 inferred exclusivity from `SINK_STATE ∈ {Exited, NotBooted, Manual}` and was wrong about `Manual`, which is a state in which a consumer may be *running*. `SAMPLE_CTR` rows and `LossCell`s inherit the single-writer property from the lane index. Payload visibility rests on the `Release`/`Acquire` cursor pair, and the consumer never reads past its observed `w` **nor advances `read` over bytes it has not staged**. Reclaim is ordered by `RETIRING` (in `boyko_diag`) being stored after the producer's last write and observed only after the consumer has drained to `write`. `ECS_HANDOFF` repeats the same argument one level up, with the producer side pinned to the `DRAIN_OWNER` holder and the consumer side to the scheduler's `ResMut` exclusivity (Decision 26).

**`Send`/`Sync`.** `LogLane: Sync` and `HandoffRing: Sync` via documented manual impls; `OnceSite`, `DynSlot`, `TargetStatCell`, `SinkSlot`: `Sync` via impls whose SAFETY blocks name the single-writer, insert-only or atomic-only argument. `TargetId`, `TargetControl`, `WarnCode`, `ErrorCode`, `PanicCode`, `Level`, `boyko_diag::SessionId`: `Copy + Send + Sync`. **`LogRing` and `LogCensus` are `Send + Sync` by a MANUAL `unsafe impl` with a SEND10-shaped argument, not by derivation** — they hold `VmColumn`, which `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70` states is **NOT `Send`/`Sync`**, while `resource.rs:42` requires both of every `Resource` (B1). The argument, its named holder set, and its `const _` `assert_send_sync` pin are in §Data structures; the load-bearing clause is that **the sink thread never touches either type** — it writes `ECS_HANDOFF`. `LogStats` is `Copy` POD and derives both. **No `!Send` handle exists** (Decision 12). `LogPod: Send + Sync` is required so a game type cannot smuggle a thread-affine value onto the sink thread.

---

## What this system can and cannot substitute for — the sync-validation confrontation

The audit established, from source, that `is_instance_extension_present(global, VK_EXT_VALIDATION_FEATURES_EXTENSION_NAME)` at `crates/boyko_rhi_vulkan/src/device.rs:2110` queries `vkEnumerateInstanceExtensionProperties` with `pLayerName == NULL`, which returns the implementation's own extensions plus implicitly-enabled layers' — never those of an explicitly-requested layer. `VK_EXT_validation_features` is supplied by `VK_LAYER_KHRONOS_validation`. Therefore `sync_validation_available` is always false, the `VkValidationFeaturesEXT` node is never chained, and `VK_VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT` is never requested. This matches the measured fact that a genuine missed barrier produced **19 messages (= baseline), zero `SYNC-HAZARD`, and a byte-identical golden**.

**A logger is a transport. It changes where a message goes and has no opinion on whether the message exists.** Therefore, explicitly:

1. **Routing validation output through `boyko_log` does not make a missed barrier visible.** Not one hazard becomes detectable. Any sentence of the form "the validation layer would have told us" remains false on this machine, before and after this plan.
2. **It would make the deadness easier to miss** — a clean, colour-coded log reads as evidence of a clean run. **And it would make the evidence droppable.** That is why v1's migration is withdrawn (Decision 9b): the channel stays synchronous, and the only change is deleting an allocation.
3. **The logger's legitimate contribution is to make ABSENCE loud.** Two mandatory mechanisms:
   - **`boyko-E2101`** — emitted at boot when validation is *requested* but the features node was not chained. A liveness claim about the channel, not about the frame.
     **Its two-sidedness is re-cut, because v2's negative leg was unbuildable here** *(fixes F2)*. v2 required a fixture "where the node is chained". By this document's own proof, `sync_validation_available` is **always false on this machine**, so that fixture cannot exist without the RHI fix the plan excludes (open question 6) — M25's disposition installed a second control this machine cannot show, which is the exact defect M25 was raised about. v3's G7 is two-sided over a predicate that **can** be driven both ways here: **(a)** a validation-**on** run ⇒ `E2101` fires; **(b)** a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`, the machine's documented switch) ⇒ `E2101` is **absent**. Both legs run today. **What G7 cannot claim**: anything about the chained case. It proves the code reports the gap when validation is requested and stays quiet when it is not; it does **not** prove the node would be chained if the extension were found. That claim belongs to the RHI fix, and `E2101` exists precisely to keep the gap greppable until then.
   - **The `LOG-CENSUS`** — at every `flush()` and at shutdown, one line per armed target: `LOG-CENSUS target=rhi level=Warn records=0 dropped=0 status=Unproven`. A target that has never delivered a record is `Unproven`, never `clean`. The vocabulary covers four ways to manufacture a silence, two of them created by the game-facing extension itself: `UnprovenLossy` (`dropped > 0`), `UnprovenSampled` (a non-zero shift — the count is `1/2^k` of the truth) and `UnprovenUnsunk` (**no `Active` sink's filter accepts this target** — a game enables a category, sees nothing, concludes clean). This is the direct translation of "a gate that cannot fail is a defect" into the logging system, extended to the new ways a game can build one.

     **There is no `vk-validation` census row, and its deletion is the point** *(fixes M4)*. v3's worked example was `LOG-CENSUS target=vk-validation … status=UNPROVEN`. But Decision 9b guarantees `debug.rs:114` is **never edited**, so **no record can ever reach a `vk-validation` target** — that row would read `records=0 status=UNPROVEN` in every run forever, whether the layer is on, off, dead, or screaming nineteen messages at stderr. A status that cannot change carries no information: it is a green-because-it-cannot-fail row wearing the exact vocabulary invented to prevent them, and it invites the *opposite* misreading — a reviewer seeing `UNPROVEN` on a run where the layer **did** print. The row is deleted; the target is not registered; and the reason is written here rather than left for a reader to infer from a row that never moves.

     **What replaces it as the liveness claim**: `boyko-E2101` and its two-sided G7 (validation-on ⇒ fires; validation-off ⇒ absent), both of which are observations that can go either way today. The census is illustrated by a target records actually reach.

     *(The alternative the review offered — feed the row from a boot-time count of messenger callbacks — was rejected because it requires editing `debug.rs:114`, and that site being untouched is a settled decision with its own byte-exactness gate. A weaker census row is a better trade than reopening a byte-frozen channel.)*
4. **The underlying fix is out of scope and belongs to the RHI** (`pLayerName = "VK_LAYER_KHRONOS_validation"`). This plan does not fix it, does not claim to, and adds `E2101` so the gap has a coded, documented, greppable identity until it is fixed.
5. **What no logger can substitute for:** an absent source. Sync-validation, the layer's `INFO`/`VERBOSE` severities (structurally excluded at `debug.rs:125-126`), GPU pipeline statistics (hard-wired to `0` at `rhi_impl/device.rs:1013`), and per-system CPU timing (does not exist) are four separate named gaps, and none is closed by this plan.

---

## Integration

### New
- **Crate `boyko_log`** — `level.rs`, `control.rs`, `target.rs`, `site.rs`, `record.rs`, `lane.rs` (`LogLane`), `codes.rs` (generated), `rate.rs` (+ `ONCE_SITES`), `sample.rs`, `sync_out.rs` (`OUT_LOCK`, `write_oracle_line` **and its durable fan-out**), `sink/{mod,console,file,binary,callback,crash,ecs,request}.rs` (`sink/ecs.rs` owns `ECS_HANDOFF`), `macros.rs`, `bin/logdec.rs`. **Deleted relative to v3**: `tsc.rs` (S4 — the clock is `boyko_diag`'s), `session.rs` (S11 — one `SessionId` mint, in `boyko_diag`), `build.rs` (S9 — the single axis is `BOYKO_PROFILE`, read by `crates/boyko_diag/build.rs`), and `report!` from `sync_out.rs` (S1).
- **Crate `boyko_diag`** — the shared substrate this plan consumes (clock, lane, loss, storage policy + `section_report`). **Specified in `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`, not here**, and jointly owned with `docs/PROFILING-SYSTEM-PLAN.md`. It lands as rung **D0**, before L0.
- **`docs/diagnostics/<code>.md`** — one per code; check 2's target; published by `doc-writer`.
- **`crates/boyko_ecs/src/ecs/core/log/`** — `LogPlugin`, `LogRing`, `LogStats`, `LogCensus`, `log_drain_system`.
- **`crates/boyko_macros`** — `#[derive(LogPod)]` (no new dependency edge: `boyko_macros` is a proc-macro crate with no `boyko_ecs` dependency).
- **`crates/boyko_log/tests/`** — `code_registry.rs` (the eight checks), `print_census.rs` (the tidy-style print ban, sharing the walker), `untested_codes.txt` and `print_allowlist.txt` (data files, each excluded from its own scan).
- **`docs/LOG-BINARY-FORMAT.md`** — the `.blog` schema with `schema_version`; the decoder refuses a mismatch.
- **`docs/HOT-PATH-EXCEPTIONS.md`** — **NO new entry.** See Invariant 1: a row for `OUT_LOCK` reds `scripts/check_hotpath_exceptions.py` because the file carries no `#[allow(clippy::disallowed_types)]` for it to match (F9).

### Migration ledger — machine-generated, not hand-tabled *(fixes M22)*
v1's table covered ~14 files against a measured **179 occurrences across 36 files** under `crates/**/src/**` (this session's grep). The migration is driven by a generated ledger, `docs/diagnostics/PRINT-CENSUS.md`, regenerated by the same walker that backs the enforcement test, with every site classified into exactly one of:

| Class | Count (measured) | Disposition |
|---|---|---|
| CLI binary stdout (`boyko_shaderdsl/src/bin/*`) | 58 | **Keep.** One crate-level `#![allow]` + rationale per bin, not per site. The only stdout writer in the workspace (S7). |
| Test-only files — **16 within-file + 7 cross-file** | 23 (`rhi_vulkan/compute/tests.rs` 16 within-file; `sdf_math/brick/tests.rs` 3 and `physics/solver/colored_tests.rs` 4 gated by a `#[cfg(test)]`-plus-`mod` declaration in the **parent**, the latter through a `#[path]`) | **Keep.** Excluded by the walker's **cross-file** `#[cfg(test)]` rule (B7). v3's within-file rule would have missed 7 of them and driven test-only sites into the allowlist. |
| Measurement lines (`runner.rs`) | **0** *(was 20)* | **Not this plan's rows** *(S1)*. `report!` is deleted; the profiler migrates all six stdout consumers to its artifact at **profiling rung 7**, which lands **before** L8b. By the time L8b runs, these producers no longer exist. |
| Validation messenger (`debug.rs:114`) | 1 | **UNTOUCHED** (Decision 9b, F12). Allowlisted with a reason, allowlist checked both ways. |
| Prose mentions inside comments (e.g. `runner.rs:560-561`) | ≥ 2 | **Not sites.** The walker's CODE stream never sees them, so they are not in the denominator (F18). |
| Everything else (production diagnostics) | the remainder | → `error!`/`warn!`/`info!` with codes. |

**The denominator, restated**: 179 raw occurrences − 58 (CLI) − 23 (test-only) = 98, minus the **20 measurement lines that S1 removes from this plan's scope** = **≤ 78** sites to disposition, of which the walker's CODE stream will resolve some number ≤ 78 as actual macro invocations. *(v2 said 83 — arithmetic error, F18; v3 said ≤ 98 — correct then, superseded by S1.)* L8c's exit criterion is `Pending` == 0 in the registry (check 3c) plus zero unclassified walker sites — two integers, both machine-produced.

**Two dependency-hygiene items the migration owns** *(S12, S2)*:
- **`boyko_demo`'s third-party `log = "0.4"`** (`crates/boyko_demo/Cargo.toml:28`, used at `crates/boyko_demo/src/main.rs:113` — verified: `log::error!("boyko_demo failed to start: {err:?}")` on the wasm arm) is **deleted at L8b** and the site becomes `error!(Demo, codes::E3001, …)`. A tidy check then asserts **no workspace `Cargo.toml` names a third-party `log` or `tracing` dependency**; re-adding one reds it. *(`env_logger` and `console_log` at `:32`/`:69` are wasm-console plumbing for that same dependency and go with it.)*
- **`crates/boyko_image/Cargo.toml:5`'s description** — "no dependency on any other workspace crate" — becomes **false** at L8a, when `boyko_image` gains `-> boyko_log` for `png.rs:206` / `inflate.rs:656`. The description is edited in the same commit; a stale description that contradicts the manifest below it is doc-rot with a two-line blast radius, and this plan is the one that creates it.

Named production files v1 omitted and that the ledger covers: `rhi_vulkan/present/targets.rs` (7), `render/texture.rs` (7), `app/{host_dump,hzb_dump,vg_census_dump,vb_probe_dump,vb_cull_probe}.rs` (14), `app/plugins.rs` (3), `app/gpu_scene/mod.rs` (3), `app/host.rs` (2), `physics/soft/self_collision.rs` (3), `ui/layout.rs` (2), `rhi/handle.rs` (2), `serialize/load.rs` (2), `ecs/asset/server.rs` (1), `ecs/ecs_master/system_api.rs` (1), `image/{png,inflate}.rs` (2), `render/{bindless,mesh_geometry_table,light_system,render_path_config,gpu_system}.rs` (8), `threadpool/worker.rs` (1), `ecs/schedule/schedule_builder.rs` (2).

### Behaviour changes worth naming

| Site | Change |
|---|---|
| `boyko_threadpool/src/worker.rs:157-168` | `abort_on_task_panic` → `error!(codes::E0201, …)` + **`flush()` before `abort()`** |
| `boyko_ecs/.../schedule_builder.rs:1334-1350` | `warn_if_empty` → `warn!(Schedule, codes::W1501, …)`; text normalised (substring-safe) |
| `boyko_ecs/.../params/diagnostics.rs:53` | `error[boyko-B0002]:` → `boyko-B0002:` (substring-safe) |
| `boyko_ecs/.../events/event_buffer.rs` | overflow emits `warn!(codes::W0701, type_name, lane, attempted, dropped)`; the `Result` is unchanged. Those four fields currently exist only inside an `EcsError` nobody reads |
| `boyko_ecs/.../query_type_registry.rs:124-144` | `warn!(codes::W0501)` at 75 % occupancy; the terminal `panic!` gains `B0502`. 1023 silent mints then a process kill is not a diagnostic |
| `boyko_rhi_vulkan/src/debug.rs:114` | **NOT TOUCHED AT ALL** (Decision 9b, F12). v2's `to_string_lossy()` removal is withdrawn: `Cow::Borrowed` means there is no allocation to remove on the normal path, and writing raw `CStr` bytes would change the emitted bytes in the invalid-UTF-8 case on a byte-frozen gate-oracle channel pinned at `boyko_app/tests/vb_bench_query_validation.rs:116-118`. Added to `print_allowlist.txt` with that reason |
| `boyko_rhi_vulkan/src/device.rs:2110` | add `error!(codes::E2101)` when validation is requested but the node was not chained |
| `boyko_rhi_vulkan/src/device.rs:3100,3158,3189` | drop `#[cfg(debug_assertions)]` → `warn!(codes::W2102)`, `RatePolicy::Once`. **Because `Once` is now per-SITE (Decision 8, F11), all three degradations report** — a code-scoped `Once` would have printed one and silently lost two. Settles the two-doctrine conflict in favour of `boyko_app/src/host.rs:228-233`'s written argument that a release-build degrade-to-disabled must be observable |
| `boyko_render/src/render_path_config.rs:311-337` | delete both hand-rolled latches (the per-frame `swap` bug) → `warn!` + `Once` |
| `boyko_render/src/light_system.rs:397,456` | delete latches → `warn!(codes::W2201, dropped_count)`; **the dropped count is now reported**, which the one-shot latch never did |
| `boyko_render/src/{bindless,mesh_geometry_table}.rs` | ad-hoc `"WARN: "` → `warn!(codes::W2202)`; keep `debug_assert!(false)`. Same per-site `Once` argument as `W2102` |
| `boyko_app` boot / teardown | owns the **whole** lifecycle (S5): `boyko_log::boot(cfg)` → `App::new` → `LogPlugin::build` → `ProfilerPlugin::build` → `Profiler::arm()`; teardown `flush_gpu()` → `Profiler::disarm()` → `flush()` → `shutdown()`, with **`flush_gpu` ahead of `flush`**. Selects a `LogRuntimePreset` (Decision 25) and takes `LANE_HOST`. Nobody else may `boot`/`shutdown`. `boyko_demo` gains a console command bound to `apply_control_spec` as the worked example of runtime control |
| `boyko_threadpool` `worker.rs:21` / `thread_pool.rs` `install` | calls `boyko_diag::set_lane` at its two existing sites (S3). This is a `boyko_diag` rung (**D1**), listed here because it is the precondition for every lane index this crate uses |
| `boyko_demo/Cargo.toml:28` + `main.rs:113` | third-party `log = "0.4"` **deleted**; the one call site becomes an `error!` (S12) |
| `boyko_image/Cargo.toml:5` | description edited: it stops being true when the crate gains `-> boyko_log` |
| `boyko_render/src/gpu_system.rs:399-404` | → `error!(codes::E2203)`. The `System` trait's missing error channel stops mattering: the logger is a side channel available from any thread |
| `boyko_image/src/{png.rs:206, inflate.rs:656}` | → `warn!(codes::W2601/W2602)`; decoding continues |
| `boyko_app/src/runner.rs` (measurement sites) | **Not this plan's rows** (S1). Migrated to the profiler's artifact at profiling rung 7, before L8b. By L8b there is nothing measurement-shaped left in `runner.rs` for this ledger to disposition |

### Enforcement *(fixes M23)*
**Primary: an in-repo tidy-style test**, `crates/boyko_log/tests/print_census.rs`, which walks `crates/*/src/**.rs`, excludes `src/bin/` and `#[cfg(test)]` regions, asserts a non-empty corpus, and fails on any `println!`/`eprintln!`/`print!`/`eprint!` outside `tests/print_allowlist.txt` — with the allowlist checked in **both** directions. We own it, and it can be shown red in one line.

**Secondary: `clippy.toml`'s `disallowed-macros`**, added only after a **shown-red canary**: `clippy.toml:21-25` records, empirically, that clippy *silently ignores a config path it cannot resolve*. The L8 gate compiles a deliberate `println!` and records the observed diagnostic in the plan's own gate log; if the key is inert on the pinned clippy, the entry is dropped and the tidy test stands alone. Independently noted: the lint cannot see `stdout().write_all`, `io::Write` on a raw handle, or `libc::write`, so it could never have carried the migration claim by itself.

### Compatibility
`Arena` / `ComponentPool` / `UnitId` untouched. `LogRing` and `LogCensus` use `VmReservation`-backed columns whose element sizes divide `COMMIT_GRANULE`, pinned by const asserts (M13, F7), **and whose `Send`/`Sync` is a manual impl with a named holder set** (B1) rather than a derivation `VmColumn` does not permit. `golden.ps1:226`'s `[vk-validation]` grep: preserved, still synchronous, **and its producer is not edited at all** (F12); `write_oracle_line` shares `stderr()`'s handle with it, so neither can splice a line into the other (S7).

**The `VB-P1d`/`VB-P4` parse contracts leave this plan entirely** *(S1)*. Nothing here writes stdout, so no golden and no parser moves **because of logging**. They do move at profiling rung 7, which owns that migration and its consequences — including the invalidation of every published floor number, since a floor measured on a different instrument bounds nothing. This document records the dependency (L8b lands after profiling rung 7) and makes no claim about the artifact channel's shape.

---

## Implementation plan

Each rung is independently green (`cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace` — **`--workspace` is mandatory**: a root `cargo check --all-targets` is vacuously green here because of the virtual-manifest quirk) and commits alone. L0-L9 are v2's ladder with the fixes folded; L10-L17 are the scope extension, each purely additive to the rung before it.

**Cross-plan preconditions and joins** *(seam record §Landing order)*. Three rungs of this ladder are no longer free-standing:

| This rung | Waits for | Because |
|---|---|---|
| **L0** | `boyko_diag` **D0** (clock, lane, loss, storage policy, `section_report`) and **D1** (`boyko_threadpool → boyko_diag`; `set_lane` at `worker_main` / `install`) | every lane index this crate uses is minted there (S3), and `GLOBAL_CEILING`/`LANE_COUNT` come from `boyko_diag/build.rs` (S9) |
| **L8b** | **profiling rung 7** (the six stdout consumers migrated to the artifact) and **7b** (floor re-measurement) | S1: L8b's 20 measurement rows do not exist because rung 7 already removed their producers. Running L8b first would leave `report!`-shaped work with no macro to do it |
| **L17** | merges with **profiling rung 14** into one joint rung **J1** | S9: one compile axis cannot be split across two rungs, and the 5 CI legs are built once |

Two joint rungs close the ladder: **J1** (= L17 + profiling 14) and **J2**, the joint baseline sitting. Until J2 lands, neither the profiler's +25 % gate nor this plan's `G10d`/`G12c` revert clauses may **fail** a rung — they record `UNPROVEN`, because a baseline taken without the other subsystem present is not a baseline for the both-present configuration (S10).

| # | What | Where | Must NOT move |
|---|---|---|---|
| **L0** | Skeleton; `Level`, `LogTarget`, `TargetId` (private field), `TargetControl` (**packed byte**), `targets!` table, `CONTROL`, `CONTROL_EPOCH_CTR`, the five macros with the 3-gate expansion, `GLOBAL_CEILING` **re-exported from `boyko_diag`** (S9 — no `boyko_log/build.rs`). **No sink, no `emit_impl`.** **Requires `boyko_diag` D0/D1.** | `src/{level,control,target,macros}.rs` | nothing exists yet |
| **L0-gate** | **G4** three-way side-effect probe: (a) compile-ceiling-below with runtime armed to Trace ⇒ 0; (b) both armed ⇒ **1000**; (c) runtime ceiling `Off` ⇒ 0. Debug *and* release. **v3's leg (d) is GONE from this rung** *(B4)*: it asserted 500 for a quantity that is 1000 (arguments evaluate at step 1, before the sample decision at step 4) **and** it needed `SAMPLE_CTR`, the seed and lanes, none of which exist before L1/L12. It reappears as **G10e** at L12-gate, over the right observable. **G2** separate build leg `BOYKO_PROFILE=off`, all three legs with their mechanisms (Decision 3). **G1 is NOT here** — it moves to L1-gate (F19). | `tests/gates_disabled.rs`, CI leg | — |
| **L1** | `LogLane` (3 partitions + `sampled_out` + `boyko_diag::LossCell`s), layout asserts, lane resolution via `boyko_diag::lane()`, retire/reclaim through the substrate, wrap protocol, **the corrected admission arithmetic (F6)**, `LogValue`/`LogArgs`, `dsp!`, `u64` loss accounting (S8), `MAX_RECORD_BYTES` runtime check, `emit_impl`. | `src/{lane,record,site}.rs` | L0's gate expansion |
| **L1-gate** | **G1** symbol gate (now that `emit_impl` exists). **G17** Error-reserve arithmetic, **three legs at TWO fill levels** (B3). Per-thread zero-alloc gate, leg (b) now asserting **`== 0`** on a fresh thread (S3); loom model of the cursor pair + `boyko_diag` reclaim; wrap-boundary proptest; **cursor-wrap-at-2³² test (E17)**; overflow test asserting `dropped > 0`; **G3** `.bss` section gate **via `boyko_diag::section_report`** (S12); **G5** distinct-`decode`-symbol upper bound (N31). | `tests/` | — |
| **L2** | `codes!` (with `prefix`, `status ∈ {Live, Pending, Historical}`), `DiagInfo`, dense `code_idx` + `CODE_IDX_EXHAUSTED`, the three code newtypes, power-of-two `EveryN` assert, per-site `Once` latch + `ONCE_SITES` push, `explain()`. **Registry seeding, itemised**: the **9 measured** grandfathered codes (`B0002`, `B1802`, `B9001`, `B9101`, `B9005`, `B9004`, `B9002`, `B1801`, `W1501`) as `Pending`; the `B9003` gap note; **`docs/diagnostics/B9004.md` and `B9005.md`, which exist in source and in NO document today** (B6); and **17 `92xx` rows as `Pending(<profiling rung>)`** so check 4 does not red on the already-committed `docs/PROFILING-SYSTEM-PLAN.md` literals (S6). | `src/codes.rs`, `docs/diagnostics/` | code numbers |
| **L2-gate** | The **eight** registry checks (integration test) over the specified three-stream walker, with **TEXT's explicit directory list excluding `docs/archive/**`** (B6) and the **cross-file `#[cfg(test)]` rule** (B7). Check 2 is `Live`-only (S6); check 3c stays disarmed until L8c. Each check **shown red once** against a deliberately broken registry; the observed failure text recorded in the gate log. **This rung commits alone** because the grandfathered codes are `Pending`, check 3b requires them to have no emitters yet (F20), and the corpus no longer contains codes nothing will ever emit. | `tests/code_registry.rs` | — |
| **L3** | `sync_out.rs` (`OUT_LOCK` per Decision 9c, `write_oracle_line` **with the durable fan-out** — B9; **no `report!`** — S1), sink thread with adaptive park, **`DRAIN_OWNER`** (B5), staged drain, console sink → `stderr()`'s own handle, `flush`/`shutdown` with `SINK_STATE`, **`PRE_FLUSH` + `sink_can_accept()`** (S5), panic-hook chaining. Timestamps come from `boyko_diag::clock`; **`RecordHeader.clock_epoch_lo` is chosen here** (S4 left the 4-byte-vs-4-bit choice to this rung; Decision 11 takes the `_pad` byte). | `src/{sync_out,sink/*}.rs` | the 20 B header assert |
| **L3-gate** | **G18** `OUT_LOCK` **three-sided** (unwind release; re-entrant completion; **durable fan-out with no console sink**). Flush-without-consumer ⇒ `NoConsumer` immediately; flush-timeout ⇒ within 2 s; shutdown detaches; **`sink_sustained_rate`** finds the drop knee (M19); **S5's four reds** (pre-boot `warn!`, post-shutdown `warn!`, `PRE_FLUSH` ordering, deferred `DiagFlag`); **S7's stderr line-integrity red** — 200 `warn!` while the validation callback fires under `cmd /c … > f 2>&1`, every `[vk-validation] ` occurrence must start a line; give `write_oracle_line` a raw fd ⇒ splices ⇒ red. *(v3's test 16, the `report!` concurrency test, is deleted with `report!` — S1.)* | `tests/`, `benches/` | — |
| **L4** | File sink + cap (`W0103`), rate limiter, `LOG-CENSUS` incl. `UNPROVEN(lossy)`, `SinkMode::Manual`. | `src/{sink/file,rate}.rs` | — |
| **L4-gate** | `Once` steady state performs **no store** (assembly/`perf` check) **and touches no shared line** (per-site latch, F11); census `UNPROVEN` at 0 records **and** at `dropped > 0`. | `tests/`, `benches/` | — |
| **L5** | ECS seam: `LogPlugin`, `LogRing` (16 B `LogLine`) on `VmReservation`, `LogStats`, `log_drain_system`, **`ECS_HANDOFF`** (B2), and the **manual `Send`/`Sync` impls with their `const _` pin** (B1). | `crates/boyko_ecs/.../log/`, `src/sink/ecs.rs` | the `COMMIT_GRANULE` divisibility asserts **and** the `assert_send_sync` pin |
| **L5-gate** | **P1, re-specified twice** (F3 → instrument; S10 → leg matrix): a **headless schedule bench**, not windowed frame time, run as a **2×2 of {logger off, on} × {profiler absent, armed}**, ABBA-counterbalanced, interleaved zero control, **one sitting**. The claim it may make is "logger-on vs off **at a fixed profiler state**", reported at both states. Baselines carry `config_tag = {profiler, logger}`; a sitting whose tag differs returns `NotResolved{ConfigMismatch}` rather than a number. | `crates/bench_bevy_vs_boyko/benches/` | — |
| **L6** | Migrate `boyko_ecs` + `boyko_threadpool`; flip those rows `Pending`→`Live`; `W1501`, `B0002` normalisation, `W0701`, `W0501`/`B0502`, `E0201`. | as tabled | `#[should_panic]` substrings |
| **L7** | Migrate `boyko_rhi_vulkan` **except the messenger, which is not touched at all**; `E2101`; `W2102` ungated in release; census wiring. | as tabled | `[vk-validation]` line, byte for byte |
| **L7-gate** | **G7, re-cut two-sided** (F2): `E2101` fires on a validation-**on** run and is absent on a validation-**off** run (`BOYKO_DISABLE_VALIDATION=1`). Channel liveness is proved separately by an **ordinary validation error from a deliberately invalid call** — the historical `mip_levels: 12` on a 512×512 image — with the **baseline of 19 messages accounted for**. A forced *hazard* is explicitly **not** the control: this machine has been measured unable to produce `SYNC-HAZARD` (M25). | `crates/boyko_rhi_vulkan/tests/` | — |
| **L8a** | Migrate `boyko_render`, `boyko_image`, `boyko_serialize`, `boyko_physics`. **Edit `boyko_image/Cargo.toml:5`'s description in the same commit** — it stops being true here. | ledger | goldens |
| **L8b** | Migrate `boyko_app`; **zero measurement rows** (S1 — profiling rung 7 removed the producers already). Delete `boyko_demo`'s third-party `log = "0.4"` and migrate `main.rs:113`; add the tidy check banning third-party `log`/`tracing` in any workspace manifest. **MUST LAND AFTER profiling rung 7 and 7b.** | ledger, `crates/boyko_demo/` | — |
| **L8c** | Check 3c armed: `Pending` == 0 (`Historical` excluded). Walker's unclassified-site count == 0 over the **≤ 78** denominator; enable `print_census.rs`; run the clippy `disallowed-macros` canary and record the result. | `tests/`, `clippy.toml` | — |
| **L9** | `boyko_ui` console widget over `LogRing`. **Deferred to the UI plan** — L16 fixes the whole contract it consumes, so nothing logging-shaped remains in it (open question 12). | `crates/boyko_ui/` | — |
| **L10** | **Dynamic targets.** `DYN_NAMES` interning, `register_dynamic_target`, `find_target`, `targets()`, the five `dyn_*!` macros, `E0106`. | `src/target.rs`, `src/macros.rs` | static-target expansion byte-for-byte; G1/G4 must still pass unchanged |
| **L10-gate** | **G8** (a-d). Bench `log_dyn_disabled` vs `log_disabled_runtime`. | `tests/`, `benches/` | — |
| **L11a** | **Downstream code tables.** `codes!` exported with `prefix`; `CodeIdx::Dynamic` + lazy minting; `codes_tidy!`; `CODE_OCCUPANCY` + `W0114`; **exhaustion behaviour + `CODE_IDX_EXHAUSTED` + `E0115`** (M3). | `src/codes.rs` | engine `code_idx` remains a compile-time constant; **no mint may ever return an aliased index** |
| **L11b** | **`LogPod`** + `#[derive(LogPod)]` generating **field-by-field `encode_pod`** (B10) + the `*_kv!` field-name forms. | `boyko_macros`, `src/site.rs` | Decision 13's structural property (asserted by test 24); **no `copy_nonoverlapping` of `size_of::<Self>()` anywhere in the derive** |
| **L11-gate** | **G9** (incl. the **exhaustion leg**, M3), **G9b** (subject changed to the padded-encode red, B10). | `tests/` | — |
| **L12** | **Sampling.** `SAMPLE_CTR`, the first-touch seed, step 4 of Algorithms A, `sampled_out` plumbing, `W0113`, census `UnprovenSampled`. | `src/sample.rs`, `src/lane.rs` | the ≤ 15 ns enabled target — **G10d decides whether this rung ships default-on** |
| **L12-gate** | **G10** (a-e), including **G10e**, the leg relocated from L0 with its observable split (B4), and the perturbation control that can flip `log-sampling` to default-off. | `tests/`, `benches/` | — |
| **L13a** | **Volume, part 1.** `Rotation`, `W0112`, `u64` loss accounting end-to-end via `boyko_diag::loss` (S8), `LogStats` u64 accumulation, `LogRing` cursor-wrap hardening **incl. `seq_lo`'s reconstruction rule** (M2). | `src/sink/file.rs`, `src/lane.rs`, ECS seam | `Rotation::NONE` remains the engine default |
| **L13a-gate** | **G11**, subject replaced by S8's fold-exactness red. | `tests/` | — |
| **L13b** | **Volume, part 2.** `BinarySink` with the widths pinned in Decision 21 (M2), the **anchor cadence** (1 s or `u32` overflow), `SITE_DICT` + full-table `W0116` + inline site records, `SINK_OUT`, dictionary records, `logdec`, `docs/LOG-BINARY-FORMAT.md`. | `src/sink/binary.rs`, `src/bin/logdec.rs` | text-sink output byte-for-byte; **the audited widths** |
| **L13b-gate** | **G12** (a-c) — **including the revert clause**. | `tests/`, `benches/` | — |
| **L14** | **Runtime sink control.** `SinkSlot` state/filter/floor, `SINK_REQ`, `request_open_file`/`request_close`, `E0107`, `ControlSource::File` + `apply_control_spec`, census `UNPROVEN(unsunk)` + `W0111`. | `src/sink/request.rs`, `src/control.rs` | no I/O on a caller thread |
| **L14-gate** | **G13** (a-c). | `tests/` | — |
| **L15** | **Crash path.** `CrashSink` opened at boot, `SINK_STATE::Exiting`, the panic-hook protocol **with step 1.5 (`PRE_FLUSH`)**, the `DRAIN_OWNER` claim (B5), `E0109`, `E0118`. **`SinkMode::Scheduled`** and its `DRAIN_OWNER` participation (B8). | `src/sink/crash.rs`, `src/sink/mod.rs` | Decision 12's flush semantics; no new unbounded wait; **`SINK_STATE` must NOT regain an exclusivity role** |
| **L15-gate** | **G14**, **three-sided** — the third leg panics while a **manual `drain()` is in flight** (B5). | `tests/` | — |
| **L16** | **Game consumption.** `TARGET_STATS`, `LogCensus`, `DiagCensus`, `LogRing::since` + `RingFilter` + `skipped`, the per-frame **`frame_epoch`** record (S11 rename), `boyko_diag::SessionId` in every header, the `ONCE_SITES` census walk (M1). | `src/target.rs`, `crates/boyko_ecs/.../log/` | the drain stays off the frame thread **except under `Scheduled`, where it is on it by design** |
| **L16-gate** | **G15**, two-sided, plus the `ECS_HANDOFF` overflow leg (`W0117` fires, `lossy` set, no silent loss). | `crates/boyko_app/tests/` | — |
| **L17 → J1** | **Merged with profiling rung 14 into ONE joint rung** (S9): the single `BOYKO_PROFILE` axis, `LogRuntimePreset`, the three header facts, and **5 CI legs** (`dev` existing + 4 net new). One axis cannot be split across two rungs. | `crates/boyko_diag/build.rs`, `crates/boyko_app/src/`, CI | G2's `off` leg must still pass unchanged |
| **J1-gate** | **G16** two-sided symbol gate + the `compile_error!` red + the three-header-fields red (Decision 25) + **P2** soak. G14/G16 cross-profile symbol censuses are CI **steps** over two legs' artifacts, not extra legs. | CI legs, `tests/` | — |
| **J2** | **The joint baseline sitting** (S10). Re-take `P1`, `P2`, `log_*` and the profiler's `zone_cost` **in the both-present configuration, in one sitting**, and stamp every baseline file with `config_tag = {profiler, logger}`. Until this rung lands, no revert clause may fail a rung — they record `UNPROVEN`. | `benches/`, baselines | — |

Ordering constraints: **D0/D1 before L0**; L10 before L11a (a dynamic target is the first consumer of a downstream code); L12 after L1; L13b after L13a (rotation is shared); L15 after L13a (the crash sink shares the file machinery); L16 after L12 and L13a (`TargetStat` carries `sampled_out`); **L8b after profiling rungs 7 and 7b**; **J1 (= L17 + profiling 14) second-to-last; J2 last**.

---

## Metrics and validation

### Benchmarks (`crates/boyko_log/benches/emit.rs`, criterion, `harness = false`)
Every row runs against a control **in the same sitting** — because this repository has measured its own wall-clock floor at 6.3 / 14.3 / 4.7 / 13.5 % across four runs of one protocol, a number without an in-sitting control is not a measurement. No benchmark binary may contain `time` / `update` / `setup` / `install` / `patch` in its name (Windows os-error-740). Never two bench jobs concurrently (`target/` once reached 74 GB and took the disk to zero, masquerading as mingw errors).

**Every bench below carries a `config_tag = {profiler, logger}` in its baseline file** *(S10)*. A sitting whose tag differs from the baseline's returns `NotResolved{ConfigMismatch}` and does **not** fail the rung — a number measured without the other subsystem present is not a number about the both-present configuration. The tags become uniform at **J2**, the joint baseline sitting.

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

**`log_disabled_compile` is deleted** (B7). A compile-disabled site optimises to nothing and the loop body is empty, so the bench measured the control against the control; it could not go red, and adding `black_box` around the arguments would falsify the very "0 argument evaluations" property under test. The property is proved by **G1** (symbol gate) and **G4** (side-effect probe), both of which have named red states.

### P1 is re-specified: the old instrument could not respond to the quantity *(fixes F3)*

v2's P1 measured **windowed frame time** with the logger on vs off. `VK_PRESENT_MODE_FIFO_KHR` is unconditional (`crates/boyko_rhi_vulkan/src/present/swapchain.rs:199`, with only a support check at `:164`), so wall-clock frame time is clamped at the refresh interval; the wall floor here is 6.893 ms. A logger idling — or even emitting 1000 records/frame at 15 ns — perturbs CPU work by ≲ 15 µs, ~0.2 % of a clamped frame. **The channel is structurally incapable of responding**, so P1's outcome was pre-determined `NOT RESOLVED`. That is the P4-6 lesson repeated: an instrument whose band is structurally zero reports `RESOLVED`/`NOT RESOLVED` without measuring anything.

Worse, open question 4 then proposed to read that silence as a positive result — "if P1 comes back `NOT RESOLVED` the thread is free" — the exact inference this document forbids three pages earlier ("absence is `UNPROVEN`, never `clean`"). Both are fixed:

- **P1 becomes a headless CPU-work bench.** `crates/bench_bevy_vs_boyko` already runs schedules with no swapchain and no FIFO clamp. P1 runs the same schedule logger-on vs logger-off, ABBA-counterbalanced, with an interleaved zero control (A0 vs A1) in the same sitting, and reports `RESOLVED(x %)` or `NOT RESOLVED` against the sitting's own floor. **What P1 cannot claim**: anything about a *windowed* frame, where present pacing dominates. That question is P2's, and P2 can only claim drift and leak, not perturbation.
- **v4 adds the second axis, because one was not enough** *(S10)*. P1 is a **2×2**: {logger off, on} × {profiler absent, armed}, all four legs ABBA-counterbalanced in **one sitting**, with the zero control interleaved. Its claim is "logger-on vs off **at a fixed profiler state**", reported at both states. The reason is not symmetry: the joint working set is 7-8 cache lines against this crate's isolated 4, so a logger-on/off delta measured with the profiler absent is a delta in a configuration a shipped frame never runs. A 1×2 P1 would have been a correct measurement of the wrong thing — which is the same class of defect as v2's FIFO-clamped channel, one level up.
- **The inference is struck.** `NOT RESOLVED` from P1 means **UNPROVEN**, and open question 4 no longer offers it as a licence. The sink thread's disposition in a shipping title is decided **structurally** — `shipping-min` exists precisely so the owner has an off switch that does not depend on a measurement this hardware may not be able to make.

### Gates — every one has a RED that can be SHOWN, and a stated limit

v2's G1-G5 carry forward unchanged in substance (G1 relocated to L1-gate). G2, G7 and P1 are re-specified above and in Decision 3. New and re-cut gates:

| # | Gate | RED variant that must be demonstrated once | **What this gate CANNOT claim** |
|---|---|---|---|
| **G17** | **Error-reserve arithmetic, three legs at TWO NAMED FILL LEVELS** *(the F6 regression gate, re-cut by B3)*. **Fill A — the reserve boundary**, `used` such that `need ≤ avail < need + ERROR_RESERVE` (e.g. `used = 15000`, `need = 40`, `CAPACITY = 16383`): (a) a further `Trace` is **refused and counted**, not written; (b) an `Error` still lands. **Fill B — genuinely full**, `used > CAPACITY − need` (e.g. `used = 16360`): (c) the lane is pre-seeded with known **undrained** records carrying a distinct byte pattern, and after the refused emit **every one of their bytes is unmodified and still decodes identically** | **replace `avail.saturating_sub(ERROR_RESERVE)` with `limit - used`** ⇒ at Fill A, `14336 − 15000` underflows to ~4.29e9, the `Trace` is admitted and **(a) fails**; at Fill B the same underflow admits a write of `need` bytes at `off = w & MASK` that runs past `read` into the seeded records and **(c) fails**. This is v2's exact code, so the gate reds on the shipped defect | **The two fill levels are not decoration.** At Fill A the broken arithmetic writes into genuinely free space — `used' = 15040 ≤ CAPACITY` — so no corruption is observable there, and (c) would be **vacuously green** if it ran at Fill A. That is precisely v3's error: leg (c) asserted an untouched *neighbouring lane* canary, which the F6 overrun can never reach, because the write offset is `off = w & MASK` and the pad/wrap rule keeps every byte inside this lane's `buf`. A cross-lane off-by-one belongs to test 6's wrap proptest, and that is where the neighbouring-canary assertion now lives — **it is deleted from G17**. G17 also cannot claim the ring is correct under *concurrent* drain; that is test 7's job. G17 drives a quiesced consumer on purpose, so the arithmetic is the only variable |
| **G18** | **`OUT_LOCK`, three-sided** *(the F8 gate, extended by B9)*. (a) A thread that acquires the lock and then panics releases it — a second thread's `write_oracle_line` completes within the deadline; (b) a re-entrant `write_oracle_line` from inside a sink panic handler **completes** and increments `OUT_REENTRANT`; **(c) the durable fan-out**: with **no console sink** and a crash sink configured, a `Warn` from a laneless thread appears in the **crash file** | replace the RAII guard with a bare `store(false)` after the write ⇒ (a) hangs and the test's own deadline reds it; restore v3's unconditional-stderr `write_oracle_line` ⇒ (c)'s crash file is empty ⇒ red | It cannot claim output is never interleaved. Under a **steal** it is, deliberately — and after B9 that includes **sync-routed records**, whose reason to exist is integrity. `OUT_STEALS > 0` in the census is the honest report; a nonzero value in a golden run is itself a defect signal. It also cannot claim durability past a `write_all`: `fsync` is `sync_durable`, opt-in |
| **G8** | **Dynamic targets, four-sided.** (a) A registered dynamic target's records arrive and appear in the census under its interned name; (b) registration past the 32-slot band returns `None` + `E0106`; (c) re-registering a name returns the same id; (d) the **bench** leg: `log_dyn_disabled` − `log_disabled_runtime` must **RESOLVE** above the sitting's floor | make `register_dynamic_target` grow past the band ⇒ (b) fails; share one slot for two names ⇒ (c) fails | It cannot claim the dynamic path is cheap enough for a hot loop — it bounds it at ≤ 4 ns disabled. **If (d) does not resolve, Decision 2's claim that gate (a) buys anything is STRUCK from this document** rather than restated |
| **G9** | **Downstream code minting, now with exhaustion** *(M3)*. (a) 16 threads mint one downstream code concurrently ⇒ exactly one dense index, no leaked counter value, `CODE_OCCUPANCY` advanced by exactly 1. **(b) exhaustion**: fill `CODE_OCCUPANCY` to `MAX_CODES`, mint once more ⇒ the mint returns `CODE_IDX_EXHAUSTED`, `boyko-E0115` fires **exactly once**, the record is **still delivered** with `Every` semantics, `codes_unindexed` advances, and **no two codes resolve to one `RateSlot`** | swap the reserve/`fetch_add` order ⇒ (a)'s density assertion fails; make the mint `fetch_add(1) % MAX_CODES` ⇒ two codes share a slot and (b)'s distinct-rate-state assertion fails | It proves nothing about a game's *registry completeness* — the eight checks are engine-scope and `codes_tidy!` must be invoked by the game. Written into the gate's own assertion message. It also cannot claim 512 is enough for a modded title; it claims only that running out is **defined, counted and never aliasing** |
| **G9b** | **`LogPod` encodes no padding** *(subject changed by B10)*. A `#[derive(LogPod)]` struct **with interior padding** (`{ a: u8, b: u32 }`) round-trips byte-identically under **Miri (Tree Borrows)** with **no uninitialised read**; `POD_LEN` equals the sum of field lengths, not `size_of::<Self>()`; a `&str` field is a **compile error** from the derive | **replace the derived field-by-field encoder with a `copy_nonoverlapping` of `size_of::<Self>()`** ⇒ Miri reports an uninitialised read on the padded struct. *(v3's red state — "drop the `POD_LEN == size_of` assert" — could only catch a **lying** `POD_LEN`; the correct-by-v3's-own-rules implementation was itself UB, and no leg covered it.)* | It cannot make `unsafe impl LogPod` safe for an arbitrary hand impl — the contract "writes exactly `POD_LEN` initialised bytes" is the hand-writer's burden. The derive is the documented route |
| **G10** | **Sampling: exactness, independence, non-perturbation — and what sampling does NOT suppress.** (a) shift = k over N records on one lane ⇒ delivered == `N >> k` **exactly** and `sampled_out == N − (N >> k)` exactly; (b) control leg shift = 0 delivers all N; (c) 8 threads × 8 targets: every (lane,target) pair independent; (d) **perturbation**: `log_enabled_0args` NOT RESOLVED vs the pre-L12 baseline; **(e) — relocated from L0-gate (B4)** — with a side-effect probe in argument position and shift = 1 over 1000 emits: **argument evaluations == 1000 AND delivered == 500**, asserted together | delete the `& mask` ⇒ (a) fails; share one counter across lanes ⇒ (c) fails; move argument evaluation behind the sample decision ⇒ (e)'s first number becomes 500 and reds | **(e) is the leg v3 got wrong twice.** It lived at L0-gate, where `SAMPLE_CTR`, the seed and the lanes do not exist (they are L1/L12) — unimplementable at its stated rung, the F19 failure. And it asserted **500 evaluations**, when the gate short-circuit means arguments are evaluated at step 1, *before* the sample decision at step 4 — the true count is 1000. Asserting both numbers in one leg is what makes the distinction legible: sampling suppresses **delivery**, never argument evaluation, and a user with a side-effecting argument needs to know that. It still cannot claim a sampled capture is *representative*: `1/2^k` is strided, not random. **If (d) resolves, `log-sampling` becomes default-off and the ≤ 15 ns row is annotated with the measured cost** |
| **G11** | **Loss-fold exactness, two-sided** *(subject replaced by S8; one gate now serves both diagnostics plans)*. Preset a lane's `LossCell` to a known value, drop N more, fold ⇒ the global `LossTotal` advanced by **exactly N** and the cell was cleared by `fetch_sub(observed)`, not zeroed | **replace `fetch_sub(observed)` with `store(0)`** and run a **live** producer ⇒ an increment landing between the consumer's load and its clear is lost ⇒ the global lags the injected count ⇒ red | It cannot claim `LogStats.dropped` is exact *between* the last fold and process death — in that window the per-lane cell is the only record, and the census says `since_last_drain` explicitly. *(v3's G11 tested `u32` saturation; with `u64` accumulation there is no ceiling to test, and the interesting property was never saturation but **fold exactness under a live producer** — which v3 did not gate at all.)* |
| **G12** | **Binary sink, three-sided.** (a) round-trip byte-identical to the text sink for the same records; (b) a record straddling a rotation appears exactly once and every rotated file decodes standalone; (c) throughput ≥ 5× the text sink in the same sitting | omit a dictionary record ⇒ (a)'s decoder cannot resolve a site; skip the dictionary re-emit on rotation ⇒ (b) fails on file `.1` | It cannot claim cross-version compatibility — the decoder **refuses** a `schema_version` mismatch. **If (c) does not separate, L13b is REVERTED** |
| **G13** | **Runtime sink control, three-sided.** (a) enabling a file sink mid-run from a non-sink thread: records before are absent, records after are present; (b) the requesting thread's **per-thread** allocation counter reads zero and no `open` occurs on it; (c) a deliberately blocking `CallbackSink` causes lane drops that are **counted** | perform the `open` on the requesting thread ⇒ (b) fails; silence the callback-stall drops ⇒ (c) fails | It cannot claim a filter change is instantaneous: a sink acts on the filter it read at the top of its current drain, so the boundary is fuzzy by up to one drain. Pinned as a property |
| **G14** | **Crash drain, THREE-sided** *(the third leg added by B5)*. (a) no consumer running, panic after an `error!` ⇒ the record reaches the crash file + `E0109`; (b) sink thread `Running` ⇒ the crash path does **not** take the role (`E0109` absent) and a per-record uniqueness check over both files shows **no record delivered twice**; **(c) `SinkMode::Manual` with a thread parked INSIDE `drain()`** (held open by a test barrier), panic on another thread ⇒ the crash path finds `DRAIN_OWNER` taken, does **not** drain, `E0109` is absent, and the uniqueness check over both files shows no record twice | replace `DRAIN_OWNER` with v3's `SINK_STATE.compare_exchange(Manual, CrashDraining)` ⇒ **(c) fails**: two consumers walk the same lanes, `STAGE`, `SITE_DICT` and `SINK_OUT`, and the uniqueness check reports duplicates (or Miri reports a data race). **v3 had no leg (c) at all** — it tested only `Running`, so the one hole it shipped was untested by construction, and it was reachable in production because `shipping-min` was the `Manual`-plus-crash-sink profile | It cannot claim survival of `abort()`, `SIGSEGV` or a guard-page stack overflow — the hook does not run (E22). It also cannot claim the crash file is complete when another consumer holds the role: that consumer writes what it staged, the rest is lost and counted. Partial mitigations named with their **real** limits (B9: a `write_all` is not an `fsync`) |
| **G15** | **In-frame consumption, three-sided.** (a) a record emitted in frame N is visible to a system reading `LogRing::since` by frame N+2; (b) it is **not** visible before the drain that consumed it; **(c) handoff overflow is loud** — flood `ECS_HANDOFF` past `HANDOFF_BYTES` in one drain ⇒ the refused lines are counted as `LossClass::Sink`, exactly one `boyko-W0117` names the count, `LogCensus.lossy` is set, and the lines that **did** fit are intact | feed `LogRing` from the emit path ⇒ (b) fails (and would have coupled the hot path to ECS storage, and falsified B1's `Send`/`Sync` argument at the same time); silence the handoff refusal ⇒ (c) fails | It cannot claim a bound tighter than "sink park interval + one frame" (one frame under `Scheduled`). `LogRingIter::skipped` reports ring-wrap loss so a console cannot silently miss lines; `W0117` reports handoff loss, which is a **different** loss and is not folded into `skipped` |
| **G16** | **Build-profile symbol gate, four-sided** *(S9)*. (a) in the `shipping` CI leg no `emit_impl` monomorphisation reachable from a `debug!`/`trace!` fixture appears; (b) in `dev` it **must** appear; (c) `BOYKO_PROFILE=shipping BOYKO_LOG_MAX_LEVEL=trace cargo build` **fails** with a named message; (d) the sink header carries `build_profile`, `runtime_preset` **and** `ceiling` as three fields, and a fixture proves `runtime_preset` and `build_profile` **can differ in one binary** | drop the `GLOBAL_CEILING` gate ⇒ the symbol appears in `shipping`; delete the `compile_error!` ⇒ (c) builds and the header prints a ceiling the profile does not name; print one profile name instead of three fields ⇒ (d) fails | It cannot claim a *dynamic* site is compiled out per-target — dynamic sites have no gate (a); they are deleted only by `GLOBAL_CEILING`, which is why the fixture includes a `dyn_debug!` site. The cross-profile census is a CI **step** over two legs' artifacts, so it cannot claim anything about a profile CI does not build (`custom`) |
| **P2** | **30-minute soak, `shipping` profile, 5 K rec·s⁻¹.** (a) `dropped == 0`; (b) resident bytes flat between minute 5 and minute 30; (c) presented frame time vs logger-off, ABBA + interleaved zero control | leak one buffer per rotation ⇒ (b) fails; shrink the ring ⇒ (a) fails, which is the intended positive control for the drop counter at session scale | (c) **cannot resolve a CPU perturbation** — FIFO clamps it (F3). P2's (c) leg is retained only as a **drift/leak** check and is labelled as such in the artifact; the perturbation question belongs to P1's headless 2×2. P2 also cannot claim anything about a different game's emission profile — the load is synthetic and the artifact records its shape. *("windowed frame time" is renamed "**presented** frame time" throughout — S11: `window` is reserved for the statistics horizon, and `os_window` for the OS object.)* |

**Where no control is possible, it is written down rather than worked around**: a forced `SYNC-HAZARD` is unavailable on this box (M25; G7 uses an ordinary validation error instead); a *chained* validation-features node is unbuildable here (F2; G7's negative leg is validation-off instead); and a hard-crash (non-unwinding) log tail is unobtainable by construction (G14's stated limit). *(v3 listed code W0101 here. S4 **deletes** it rather than allowlisting it: an uncontrolled code in one plan and a controlled `boyko-W9207` in the other, for one condition, is two answers to one question. `tests/untested_codes.txt` loses the row.)*

### Mandatory tests
1. **G4 — three-way gate separability** (§L0-gate). Each gate has its own red state; the enabled leg must reach **1000**, not merely `0` when disabled. *(v3's fourth leg — "shift = 1 ⇒ exactly 500" — is **deleted from this test** and reappears as **G10e** at L12-gate, over a split observable: 1000 argument evaluations **and** 500 delivered. B4.)*
2. **G1 — symbol gate** (at **L1**-gate, not L0 — F19). Disabled fixture: no `emit_impl` symbol. Armed fixture: symbol present. Red state: delete a gate.
3. **Allocations on the producer path — steady state 0, first emit 0** *(F26, tightened by S3)*. Via a **per-thread** counting allocator (`thread_local! { static N: Cell<u64> = const { Cell::new(0) } }` — const-init, no `Drop`, no TLS registration, no allocation of its own). Three legs:
   - **(a) steady state**: arm *after* a warm-up emit on the same thread; assert exactly **0**. Red state: make `encode` allocate.
   - **(b) first emit**: arm on a **fresh** thread *before* its first emit; assert **`== 0`**. v3 asserted `≤ 1` and recorded the number, because its lane guard was a `thread_local!` with a `Drop` and destructor registration allocates on some platforms. **S3 deletes that guard** — `boyko_diag::lane` is a `Cell<u16>` with no `Drop` — so the allowance has no source left and the leg becomes exact. **Red state**: reinstate a `Drop`-carrying TLS guard ⇒ the count becomes 1 ⇒ red. An assertion of `≤ 1` would have been green either way, which is why it is now `== 0`.
   - **(c) monotonicity**: 1000 emits on that fresh thread must not raise the count above leg (b)'s value. A per-emit allocation would show here even if leg (a)'s warm-up hid it.
   This covers `SinkMode::Thread`, which the process-global counter structurally cannot: `crates/boyko_ui/tests/zero_alloc.rs:44-60` had to add `ARM_LOCK` after observing an impossible **negative** delta from a sibling thread, and a permanently resident sink thread cannot be serialised by a test-local lock (M18). The process-global variant is retained as a second, `Manual`-mode gate with its limitation stated.
4. **Overflow drops and counts** — fill a lane, assert `dropped > 0`, exactly one `W0102` per drain with matching counts.
5. **Error reserve** — flood with `Trace`, assert a subsequent `Error` still lands.
6. **Wrap protocol** — proptest over record sizes crossing every tail offset in `LANE_BYTES-32 ..= LANE_BYTES`; assert no byte is written outside the lane (**poison the neighbouring lane's guard bytes and check them** — this is where the cross-lane canary belongs, because an off-by-one in the wrap rule *can* cross a lane boundary, whereas the F6 admission defect structurally cannot; B3), and that producer and consumer agree on every PAD.
7. **Staged drain under a LIVE producer** (B1's red state) — a producer running at full rate while the sink drains; assert every decoded record is byte-identical to what was offered. **v1's design fails this test; v1's tests never ran it, because both drove a quiesced producer.**
8. **Lane claim/retire, and the join** — 200 short-lived threads against `LANE_COUNT` lanes; assert every spare eventually returns to `FREE`, **and** that `Warn`/`Error` from unlaned threads reached the synchronous fallback **at its durable destination** (M26 + B9). Reclaim is asynchronous, so the assertion is "eventually, within a bounded flush", not "immediately". **Plus S3's three reds**: (a) a zone emitted on worker *k* lands in lane *k* and nowhere else — delete the `set_lane` call in `worker_main` ⇒ every worker reads `LANE_UNCLAIMED` ⇒ red; **(b) the JOIN red** — one fixture emits one `warn!` and opens one profiler zone on the same worker; the log record's lane field and the sample's lane index must be **the same integer**; give the logger its own registry back ⇒ they differ ⇒ red; (c) per-thread allocation count on first emit is **0** — reinstate the `Drop` guard ⇒ 1 ⇒ red.
9. **Flush without a consumer** returns `NoConsumer` immediately; **flush timeout** returns within 2 s with `E0105`; **shutdown** detaches on timeout with `E0108`.
10. **Panic hook flushes** — `catch_unwind` around a panic after an `error!`.
11. **Registry: the eight checks**, each shown red once during development, over the three-stream walker.
12. **Rate policies** — `Once` (incl. the no-store property), `EveryN`, `MinInterval`, `suppressed_since_last`.
13. **Census** — `UNPROVEN` at 0 records **and** `UNPROVEN(lossy)` at `dropped > 0`.
14. **Miri (Tree Borrows)** — ring, claim CAS, typed header round-trip incl. `*const LogSite` provenance, staged copy.
15. **loom** — claim/retire and the cursor pair. (Loom *release* binaries crash at startup on this box, pre-existing; run loom in debug.)
16. **DELETED** *(S1)*. v3's test 16 pinned `report!`'s output byte-identical to the `VB-P1d`/`VB-P4` lines under concurrency. `report!` no longer exists and this plan writes no stdout, so the property has no subject here; the equivalent obligation moves to the profiler's artifact channel and to the S1 grep gate (`rg 'VB-P1d |VB-P4 pass=|VB-SV0-S1\.5 ' crates/*/src` ⇒ zero after profiling rung 7). **Replaced, not merely removed**, by S7's **stderr line-integrity** test at L3-gate: emit 200 `warn!` while the validation callback fires, under `cmd /c … > f 2>&1`; assert **every** `[vk-validation] ` occurrence starts a line. Red state: give `write_oracle_line` a raw-fd `write` ⇒ it splices into a messenger line ⇒ red. The number is deliberately not reused for another test, so a reader of an older review can still find what happened to it.
17. **`[vk-validation]` liveness and byte-exactness — with a POSITIVE control, because zero messages is this machine's measured normal** *(fixes F1)*. v2's test 17 said "`golden.ps1`'s grep matches, and the message is on the wire before the frame returns" — but `golden.ps1:226` matches nothing and `:232` prints "clean (0 messages)" in green at **zero**, which is the steady state here (a genuine missed barrier produced zero messages, twice). It could not distinguish "the prefix survived" from "no message existed", and its second clause named no observation mechanism at all. v3:
    - **(a) positive control**: a fixture makes a deliberately invalid call that produces ≥ 1 ordinary validation message. Assert `count ≥ 1` and that the line begins with the byte-exact `"[vk-validation] "` **including the trailing space**, pinned against `crates/boyko_app/tests/vb_bench_query_validation.rs:116-118`'s constant.
    - **(b) ordering, with a mechanism**: immediately after the offending call returns, the fixture writes a synchronous marker line. Assert the `[vk-validation]` line **precedes** the marker in the merged stream. Red state: buffer the messenger ⇒ the marker comes first.
    - **(c) negative**: a run with no invalid call produces zero `[vk-validation]` lines. *(v3's (c) also asserted a census row reading `status=UNPROVEN` for a `vk-validation` target. **That clause is struck** — M4: no record can ever reach such a target, so the row was inert by construction and is deleted from the census. What (c) asserts is the absence of the lines, which is an observation about the messenger's own channel.)*
    - **What test 17 cannot claim**: that a run with zero messages is a run with no defects. It never could; §sync-validation confrontation is the standing statement of that.
18. **Dynamic target interning** — 32 registrations succeed with distinct ids; the 33rd returns `None` with `E0106`; re-registering an existing name returns the same id; concurrent registration of one name from 16 threads yields one id.
19. **Cursor wrap at 2³²** (E17) — preset `write`/`read` to `u32::MAX − 64`, push records across the boundary, assert every record decodes and `write.wrapping_sub(read) <= LANE_BYTES` throughout.
20. **`LogRing` cursor wrap and `seq_lo` reconstruction** *(extended by M2)* — same treatment for `head` / `arena_cursor`, with `skipped` reported; **and** a leg that drives `seq` past `2³²` and asserts that `since(cursor)` returns the correct records for cursors on **both** sides of the `seq_lo` wrap, using the Decision 21 reconstruction rule. Red state: reconstruct with a plain `as u64` widening instead of the wrapping-difference rule ⇒ every record before the wrap is reported as newer than every record after it.
21. **Loss-fold exactness under a live producer** (E18) — see G11. *(v3's "saturating drop counters" is gone with the saturation, S8.)*
21b. **`ECS_HANDOFF` overflow** — see G15 (c): overflow is counted as `LossClass::Sink`, `W0117` fires once per drain, `lossy` is set, and the lines that fit are intact.
22. **Sampling exactness and independence** — see G10 (a)(b)(c).
23. **Downstream code minting under contention** — see G9.
24. **`LogPod` under Miri (Tree Borrows)** — a padded `#[repr(C)]` struct round-trips **with no uninitialised read** (B10: the derive encodes field-by-field, so padding never enters the record); a `fmt_pod` reading beyond `POD_LEN` is caught; **and an assertion that no user code executes between lane acquisition and the `Release` store** (a `LogPod` whose `fmt_pod` sets a TLS flag; the flag must be unset at the `Release` store and set only during drain).
25. **Sink filter and `UNPROVEN(unsunk)`** — a target enabled at `Info` with no sink accepting it produces `status=UNPROVEN(unsunk)` + `W0111`, not silence.
26. **Runtime sink open/close** — see G13.
27. **Crash drain, both sides** — see G14.
28. **Binary round-trip and rotation** — see G12 (a)(b).
29. **`EPOCH` correlation** — every record's `tsc` falls between the epochs of exactly one frame; a record emitted during the drain itself is attributed to the next frame, and that is **asserted, not assumed**.
30. **Control spec parsing** — `apply_control_spec("net=debug/6!, ecs=off")` sets level, shift and sync bit for the named targets, bumps `control_epoch()` by exactly 1, leaves unnamed targets **bit-identical**, and rejects an unknown name with a coded error rather than silently ignoring it.
31. **Per-site `Once`, and its census row** *(F11, extended by M1)* — three sites sharing one code, all three fire exactly once; the same site called 10⁶ times fires once and performs **no store** after the first; **and the census prints three `LOG-ONCE` rows, one per fired site, each with a real `fired=1`** — with a fourth, never-called site **absent** from the list. Red state: restore v3's per-code `fired=1` literal ⇒ the three-site case prints one row and the count is fiction.
32. **Clock epoch straddle** *(S4)* — inject a synthetic forward jump into `boyko_diag::clock` mid-run; assert the log records emitted after the jump carry the **incremented** `clock_epoch_lo` and that the sink renders it. Red state: give this crate its own `ticks_per_ns` back ⇒ its rendered wall times drift by the injected amount while the profiler's window is quarantined ⇒ the cross-check reds.
33. **Crate-graph tidy** *(S2, S12)* — `crates/boyko_utils/Cargo.toml` has an **empty** `[dependencies]`; `crates/boyko_log/Cargo.toml` names exactly `boyko_diag` and `boyko_macros`; **no** workspace manifest names a third-party `log` or `tracing`. Red state: re-add `log = "0.4"` to `boyko_demo` ⇒ red.
34. **Registry corpus composition** *(B6)* — assert the walker's TEXT corpus **excludes** `docs/archive/**` and that `files_scanned ≥ 500` with the `boyko-W1501` sentinel present; assert `docs/diagnostics/B9004.md` and `B9005.md` exist with all three sections. Red state: add `docs/archive/**` to the corpus ⇒ check 4 reds on `B9000`/`B9003`/`W9003`, which have no rows and never will.

### Property-based
- Random `(level, target, arg-tuple)` sequences round-trip byte-identically through `encode`/`decode`, **including `LogPod` members**.
- Random fill/drain interleavings: `emitted == drained + dropped + sampled_out`, exactly, always. *(`sampled_out` is a separate term precisely so this identity stays exact — folding it into `dropped` would have made the drop count a liar in the other direction. With `u64` accumulation the identity holds at session scale too, which the saturating `u32` could not promise.)*
- For any `seq` and any retained-line set, the Decision 21 `seq_lo` reconstruction returns the true `u64` sequence number, including across the `2³²` boundary (M2).
- For any rotation schedule and any record stream, `logdec` over all retained files yields a subsequence of the emitted stream with no duplicates and no reordering within a file.
- For any control-spec string, `apply_control_spec` is idempotent: applying it twice yields bit-identical `CONTROL`.
- **For any `(used, need, level)` with `used <= CAPACITY`, the admission arithmetic never admits a write that would pass `read`** — the property F6 violated, stated as a proptest over the raw integers rather than only as a scenario test.

### `debug_assert!` invariants
`len <= MAX_RECORD_BYTES`; `len == HEADER_BYTES + args.encoded_len()`; `write.wrapping_sub(read) <= CAPACITY`; `boyko_diag::lane() < LANE_COUNT || == LANE_UNCLAIMED`; `!IN_EMIT.replace(true)` (re-entrancy); **`DRAIN_OWNER == my_token` for the whole of any drain body**; `code_idx < MAX_CODES || == CODE_IDX_EXHAUSTED`; `boot()` at most once; `codes!` strictly increasing (also a compile-time `const _`); `EveryN(n).is_power_of_two()` (compile-time); `sample_shift <= 15`; **`LogPod::POD_LEN == Σ field lengths` (compile-time, generated by the derive — B10)**; `SAMPLE_CTR` row index == `boyko_diag::lane()`; `SINK_STATE` transition is a permitted edge; **`ECS_HANDOFF` is written only by the `DRAIN_OWNER` holder**. *(`TargetId < MAX_TARGETS` is **not** in this list: it is a type invariant upheld by a closed constructor set, so there is nothing to assert at use — F15. `LogPod::POD_LEN == size_of::<Self>()` is **gone**: it was the assertion that made the padding UB look checked.)*

**Release-live** (these run in every profile, not only debug): the `MAX_RECORD_BYTES` check, the admission-control `saturating_sub`, the sampling arithmetic, the census status computation, **the `DRAIN_OWNER` CAS on every consumer entry** (B5 — an exclusivity proof that only holds in debug is not a proof), `OUT_LOCK`'s acquire deadline, the `SINK_REQ`-full refusal, the `ECS_HANDOFF` admission check, and the code-index exhaustion branch. *(v3 listed "the drop-counter saturation guard"; there is no saturation any more — S8.)*

---

## Edge cases

| # | Case | Behaviour |
|---|---|---|
| E1 | Log before `boot()` | `CONTROL` is `.bss`-zero = level `Off`, shift 0, sync 0; one L1 load, one `and`, not-taken branch. Correct and free — **for `Info`/`Debug`/`Trace`. `Warn`/`Error` additionally consult `sink_can_accept()` and take the SYNCHRONOUS channel** rather than vanishing (S5). One extra load + branch on the failed-gate path of those two levels only; bounded by the `log_disabled_warn ≤ 4 ns` row. |
| E2 | Log after shutdown | Same as E1 for `Warn`/`Error`: `sink_can_accept()` is false in `Exited`, so they take the synchronous channel (S5). Lower levels land in lanes that nothing drains; `dropped` climbs **in `u64`, without ever saturating** (S8); census reports lossy. Shutdown flushes first. |
| E3 | Record over `MAX_RECORD_BYTES` | **Runtime** check, every profile: dropped, `TOO_LARGE` flag, counted. Not a debug panic reachable from safe code (N29). |
| E4 | Ring exactly full | One-slot-reserved convention distinguishes full from empty without a third variable. |
| E5 | Tail too short for a record | PAD record if `tail >= HEADER_BYTES`, else the shared implicit-wrap rule. Both sides apply the same rule (B3). |
| E6 | 81st concurrent logging thread (33rd in `shipping`) — the spare band is exhausted | `Warn`/`Error` go to the synchronous fallback **at its durable destination**, so the promise is not inert in `shipping`/`shipping-min` (M26 + B9); lower levels count as `LossClass::Unclaimed`; census reports it. Sampling is skipped entirely for an unlaned record, because `SAMPLE_CTR` has no row for it — which is why the lane is resolved **before** the sample step (B4). |
| E7 | Thread dies mid-record | Impossible to publish a partial record: header+payload write and the `Release` store are straight-line with no yield point. A thread killed between them leaves `write` unmoved; the bytes are overwritten. |
| E8 | `tsc` wrap | 64-bit invariant TSC at ~3 GHz wraps in ~195 years. Not handled; stated. |
| E9 | Non-monotonic clock across sockets | Merge order degrades to approximate — already the documented property. Single-socket assumption stated. |
| E10 | `&str` > 256 B | Truncated, `STR_TRUNCATED`, sink appends `…[truncated]`. |
| E11 | Two engine targets claim one ID | **Does not compile** (Decision 15). Downstream collision ⇒ `boyko-E0104` at boot, naming both. |
| E12 | File sink hits `max_bytes` | `Rotation::NONE` (engine default): one `boyko-W0103`, writing stops, other sinks continue. `Rotation{keep}`: rotate, delete the oldest, re-emit anchor + dictionary, `boyko-W0112` **naming the deleted file and the record range lost**. |
| E13 | Validation storm | Unchanged from today: `eprintln!` on `stderr()`'s own lock, synchronous, never dropped (Decision 9b — the site is not edited). |
| E14 | Panic inside a sink | The sink catches, direct-writes, continues — and the direct-write **cannot self-deadlock**, because `OUT_LOCK` detects same-thread re-entrancy and writes with a leading newline instead of spinning (Decision 9c, G18b). A sink must not kill the process. A sink that faults repeatedly is set `Faulted` and skipped; other sinks continue. The `DRAIN_OWNER` guard is RAII, so an unwind out of the drain body **releases the consumer role** rather than stranding it (B5). |
| E15 | `flush()` from two threads | Distinct sequence numbers via `FLUSH_SEQ.fetch_add(AcqRel)`; both return. |
| E16 | Drop order at shutdown | `flush_gpu()` → `Profiler::disarm()` → `shutdown()` → `PRE_FLUSH` callbacks → `flush()` → `Exiting` → unpark → bounded spin on `SINK_EXITED` → detach on timeout → sinks close. **`flush_gpu` ahead of `flush` is the whole fix** for GPU-side diagnostics emitted after the logger stopped accepting them (S5). Idempotent; safe from `App` teardown, the panic hook and process exit. |
| **E26** | **A second `drain()` while one is in flight** | The second caller's `DRAIN_OWNER` CAS fails and it returns `DrainResult::Busy`. **Not a `debug_assert`**: a second manual caller is a user error, not a bug in this crate, and a debug-only assertion would have been silent in release — which is exactly how v3's `Manual` hole stayed invisible (B5). |
| **E27** | **A panic while a manual or scheduled drain is in flight** | The crash path's CAS fails; it does **not** drain. Records the other consumer has staged are written by **it**; the rest stay in the lanes and are lost, counted, and reported. v3 would have started a second consumer here (B5, G14 leg (c)). |
| **E28** | **`ECS_HANDOFF` fills during a formatting storm** | The producer refuses, counts `LossClass::Sink`, and the drain emits one `boyko-W0117` naming the count; `LogCensus.lossy` is set. The **byte sinks still have every record** — only the in-frame view is short, and a HUD that reads `lossy` cannot present its count as a total (D26, G15c). |
| **E29** | **A ninth `PRE_FLUSH` registration** | `Err(PreFlushFull)` + `boyko-E0118`. Eight is a hard cap, chosen so the array is a `.bss` const-extent static; a ninth registrant is a design question, not a runtime growth event (S5). |
| **E30** | **The code-index space is exhausted** (512 minted) | The mint returns `CODE_IDX_EXHAUSTED`; the record is **still delivered**, with `Every` semantics and no rate state; `boyko-E0115` fires once; `codes_unindexed` counts every subsequent one. **Never an aliased index** — sharing a `RateSlot` would apply an unrelated subsystem's `EveryN` state (M3). |
| **E17** | **Lane cursor wraps `u32`** (~2.4 h at 500 KB·s⁻¹·lane) | **Correct, and proved**: every comparison is `wrapping_sub`, every index is `& MASK`, and `w − r ≤ CAPACITY ≪ 2³¹` so the unsigned difference is unambiguous across one wrap. Test 19 presets the cursors at the boundary. |
| **E18** | **Drop counting at session scale** | `boyko_diag::LossCell` is a plain `u64` written by the lane owner (no lock prefix), folded into an `AtomicU64` with `fetch_sub(observed)`. **~8 800 years to wrap at 66 M offers·s⁻¹**, so there is no ceiling state and no `SATURATED` token — a token a reader could never *compare* stops existing (S8). What the census still says explicitly is `since_last_drain`, because between the last fold and process death the per-lane cell is the only record. |
| **E19** | **33rd dynamic target** | `register_dynamic_target` returns `None` and `boyko-E0106` names the rejected string. There is no `TargetId` to misuse afterwards, because absence is `Option<TargetId>` (F15) — the "emitted on an invalid id" case is unrepresentable rather than counted. |
| **E20** | **A target is enabled but no sink accepts it** | Census `status=UNPROVEN(unsunk)` + `boyko-W0111` once. Without this, a game enables a category, sees an empty log, and concludes "clean" — the vacuous gate in a new costume. |
| **E21** | **Rotation deletes evidence** | `boyko-W0112` names the deleted file and the record range lost. Every retained file is independently decodable (anchor + dictionary re-emitted). A capture that silently discards its own beginning is not a capture. |
| **E22** | **Hard crash — `abort()`, `SIGSEGV`, guard-page stack overflow** | The panic hook does not run; whatever is in the lanes and in `SINK_OUT` is lost. **No design in this crate survives it**; making every record durable is a syscall per record. Bounded, not fixed, by three mitigations **whose limits are now stated rather than implied** (B9): (i) the per-target sync bit — `write_oracle_line` fans out to the boot-opened crash file, so in `shipping`/`shipping-min` the bytes actually leave the process instead of going to an unconfigured stderr; **"durable" here means one `write_all`, not `fsync`**, which is opt-in via `sync_durable` at ~0.1-10 ms; and under `OUT_LOCK` contention the cost is bounded only by the 50 ms steal deadline, at which point the line may interleave. (ii) `flush_interval_ms` (default 1000 in `shipping`). (iii) The boot-opened crash file, which at least exists and carries the session header **and, under `Scheduled`, holds records adjacent to the crash rather than to boot** (B8). Stated, not worked around. |
| **E23** | **Sampled capture aliases a periodic emitter** | `1/2^k` is strided, not random. Per-lane seeding breaks aliasing across lanes, not within one. The census prints `sampling=1/N (strided, not random)` and `boyko-W0113` fires once per sampled target, so the bias is in the log rather than only in a footnote. |
| **E24** | **A `CallbackSink` blocks** | The sink thread stalls; lanes fill; drops are counted and reported. This is the stated cost of putting a network/telemetry sink behind the callback seam rather than inside `boyko_log` (§Refused), and G13c demonstrates it rather than leaving it as prose. |
| **E25** | **`OUT_LOCK` acquire times out** | The writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not (Decision 9c). |

---

## Open questions

1. **STRUCK — `report!` no longer exists** *(S1)*. v3 asked whether `report!` should gain a schema-versioned TOML form. The measurement channel is the profiler's end to end and its output is an artifact, so the question moves to `docs/PROFILING-SYSTEM-PLAN.md` in a stronger form (the artifact *is* schema-versioned) and is not this plan's to ask.
1b. **Retail diagnostics footprint — VALUES call, owner.** The joint retail figure is **1.95 MiB** (this crate 1.16, the profiler 0.85), against a profiling headline of "≤ 1 MiB retail" the owner may have read as the *whole* diagnostics budget. Reducing it means cutting one of: this crate's 32 × 16 KiB lanes (512 KiB), `SINK_OUT` (256 KiB), or the profiler's dynamic-zone arenas (96 KiB). How much does a shipped title pay for diagnostics?
1c. **`shipping-min` semantics — SCOPE call, owner.** This plan's `ShippingMin` now has no resident *logging* thread (`SinkMode::Scheduled`, B8). The profiling plan's `Always` tier still writes a telemetry stream **synchronously on the dispatcher** in the same profile, so a title that chose `shipping-min` to avoid resident diagnostics still pays a per-window `write_all`. Keep, or make `shipping-min` also disable telemetry?
2. **`--explain` delivery.** This plan embeds a one-line `summary` and requires `docs/diagnostics/<code>.md` (check 2). It does not add a `boyko-explain` binary. rustc's registry stays honest partly *because* three consumers read one table; we have two. Does the owner want the third?
3. **`.bss` budget.** Decision 3's matrix: **≈ 2.90 MiB** reserved for `dev` (v3 said ≈ 3.4; S3 cut lanes 128 → 80 and B2 added a 256 KiB handoff), **≈ 1.15 MiB** for `shipping`, demand-zero, resident a small fraction. Acceptable, or should `dev` default to 64 × 8 KiB lanes? *(See also 1b: in isolation this is a small number; jointly the `dev` total is ≈ 9.58 MiB.)*
4. **Sink thread in shipping builds.** One extra OS thread, idle-parking at 125 Hz. **v2's inference here is struck** *(F3)*: it said "if P1 comes back `NOT RESOLVED` the thread is free", which is the reading of silence as proof that this document forbids everywhere else. `NOT RESOLVED` means **UNPROVEN**. The question to the owner is therefore a VALUES call and not a measurement: is one parked OS thread acceptable in a shipped title, given that `ShippingMin` exists with **no resident logging thread** — paying instead a bounded per-frame drain on the frame thread and a stated hole around boot/shutdown (Decision 10, B8)? *(v3 offered `Manual` here, which had no consumer at all and was therefore not a real alternative.)*
5. **The `15xx` / `90xx` block split** is grandfathered and tidying is forbidden. Confirm — renumbering breaks the book, the `#[should_panic]` assertions and the never-reuse rule at once.
6. **The one-line RHI fix for sync-validation** (`pLayerName = "VK_LAYER_KHRONOS_validation"`) is deliberately not in this plan. It has a large blast radius: sync-validation coming alive would surface real hazards and could turn every golden run red. Pull it into L7, or keep it a separate RHI item coded by `E2101`? **Note that G7's negative leg is re-cut around its absence** (F2), so answering this does not block L7.
7. **`OUT_LOCK`'s registration is now moot** *(resolved by F9)*. v2 asked the owner to confirm a `docs/HOT-PATH-EXCEPTIONS.md` entry. That entry is **not implementable**: `scripts/check_hotpath_exceptions.py` matches rows against `#[allow(clippy::disallowed_types)]` counts per file, and an `AtomicU64` carries none, so the row would red CI. The question that remains is narrower: **confirm the steal-on-timeout trade** (Decision 9c) — a possibly-interleaved line instead of a possible hang — rather than an attempt to make the channel lock-free, which is strictly more machinery for a path that is cold by construction.
8. **Dynamic band size.** 32 slots (IDs 224..=255). A modding-heavy title could want more, but every extra slot comes out of the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and `TARGET_STATS` are all sized by; past 256 those three arrays become two-level structures. **Recommendation: ship 32 and treat "more than 32 data-defined categories" as a signal that the taxonomy belongs in source.** VALUES call.
9. **Is `log-sampling` default-on?** Decided by G10d, not by this document. Flagged so the answer is recorded rather than absorbed.
10. **Does the engine profile want rotation?** `Rotation::NONE` is kept as the engine default so a bench cannot silently discard its own beginning. A long editor session would want rotation. **Recommendation: `dev` keeps `NONE`; a future `editor` profile gets rotation.** SCOPE call.
11. **Telemetry payload shape.** `LogCensus` gives a game per-target counts; the binary log gives it everything. What it does *not* give is a compact per-session summary suitable for upload (a few hundred bytes). That is a game-side reduction over `LogCensus`, deliberately not designed here — but if the owner wants a canonical shape shipped, it is a small additive rung after L16.
12. **Should `boyko_ui`'s console (L9) live in this plan or the UI plan?** L16 now fixes the entire contract it consumes (`since`, `RingFilter`, `skipped`, `LogCensus`, `CONTROL_EPOCH`), so L9 is a pure UI rung with no logging design left in it. Recommend moving it wholly to the UI plan and deleting L9 from this ladder.

---

## Checklist

**Structure** — goal in perf+functional terms, both audiences ✔ · concrete targets with named red-state controls ✔ · every decision justified via perf/cache/parallelism ✔ · alternatives rejected with reasons ✔ · trade-offs listed ✔ · the five audience conflicts named, decided, and costed for the losing side ✔ · eight asks explicitly refused with reasons ✔ · **the joint (both-subsystems-present) cost stated where the reader meets each number, not only in a table** ✔
**Data structures** — every field typed + commented ✔ · `repr`/`align`/`packed`/`transparent` where it matters ✔ · three-partition cache-line split ✔ · sizes pinned by `const _: () = assert!`, **including the `COMMIT_GRANULE` divisibility pin that v2 violated** ✔ · **`Send`/`Sync` pinned by a `const _` `assert_send_sync`, because `VmColumn` is neither** ✔ · false-sharing padding specified for `LogLane`, `HandoffRing`, `RateSlot`, `DynSlot`, `TargetStatCell`, `SinkSlot` ✔ · **the sink→ECS handoff is a specified structure with layout, capacity, ordering, overflow and a budget row** ✔ · producer working set ≤ 4 lines in isolation, **7-8 jointly, both stated** ✔
**API** — minimal ✔ · no internal types in signatures ✔ · lifetimes trivial ✔ · no `dyn` anywhere (the callback seam is an `extern "C" fn` + ctx) ✔ · generics where specialisation is needed (`emit_impl<A: LogArgs>`, the `LogPod` blanket) ✔ · **no public type can hold an out-of-range index** ✔ · **no encode path reads an uninitialised byte** ✔
**Multithreading** — model explicit ✔ · every atomic's ordering stated, including the new data ✔ · **every wait bounded, including `OUT_LOCK`'s** ✔ · the one sync point short-circuited when no consumer exists ✔ · **the consumer role is CAS'd directly (`DRAIN_OWNER`), not inferred from a state that merely correlates with it** ✔ · `Send`/`Sync` consistent **and argued for the two types that cannot derive it** ✔ · race-freedom argued, including the single-thread re-entrant case, `LogPod` and the handoff ✔
**Correctness** — 30 edge cases ✔ · **the admission arithmetic proved by induction and proptested over raw integers** ✔ · session-scale integer audit with a per-quantity table **covering `seq_lo` and every `BinarySink` width** ✔ · lane identity single-sourced ✔ · drop order (E16) ✔ · `unsafe` invariants in the `Sync` SAFETY blocks (clauses 1-4 incl. 1c-1f, plus the handoff's four) and per algorithm ✔
**Integration** — machine-generated ledger, not a hand table ✔ · census arithmetic reconciled and its denominator named (**≤ 78** after S1) ✔ · API changes explicit ✔ · `Arena`/`ComponentPool`/`UnitId` untouched ✔ · **this plan writes no stdout at all** ✔ · 22 rungs plus 2 joint rungs, each landing green and committing alone, each with a "must NOT move" column ✔ · **every cross-plan precondition named with the rung it waits for** ✔
**Validation** — 34 tests ✔ · 6 property families ✔ · 13 benches with in-sitting controls **and a `config_tag`** ✔ · `debug_assert!` **and release-live** invariant lists ✔ · every gate has a named red state **and an explicit "what this gate CANNOT claim"** ✔ · three gates (G8d, G10d, G12c) can **revert their own rung** ✔ · **one gate (G17) exists to red on a defect this document previously shipped, at two named fill levels because one of them is vacuous** ✔ · **three v3 gate legs deleted or relocated for being unable to go red** (G17c, G4d, the `vk-validation` census row) ✔

---

## Findings disposition (v1 → v2 review) — carried forward unchanged

| # | Finding | Disposition | Where in v2 |
|---|---|---|---|
| **B1** | Drain frees the ring before it reads it | **FOLDED** | §Algorithms C — staged typed-header + payload copy, `read.store` moved after staging; test 7 drives a **live** producer (v1's tests could not see it) |
| **B2** | `const ENCODED_LEN` undefined for `&str`/`fmtv` | **FOLDED** | §Decision 1a — `encoded_len(&self)`; `fmtv` deleted (see B4) |
| **B3** | Ring never wraps | **FOLDED** | §Algorithms A5 — non-straddling records, shared wrap rule, PAD sentinel; test 6 proptests every tail offset |
| **B4** | `fmtv` makes emit re-entrant on one thread | **FOLDED** | §Decision 13 — `fmtv` deleted, `dsp!` runs in argument position; SAFETY clause 1b; `IN_EMIT` debug guard |
| **B5** | L0 gate cannot fail; probe is one-sided; gates not separable | **FOLDED** | §L0-gate G4 — three separable legs, enabled leg must reach 1000; G2 separate build leg for `GLOBAL_CEILING` |
| **B6** | Registry check 3 vacuous; check 5 self-defeating; no corpus assertion | **FOLDED** | §Decision 6 — check 3 scans `.rs` only, excludes `docs/` and `codes.rs`; allowlist moved to a data file excluded from its own scan; check 0 asserts corpus size + a pinned sentinel |
| **B7** | `log_disabled_compile` bench cannot fail | **FOLDED** | §Metrics — bench deleted; replaced by G1 symbol gate + G4 probe |
| **B8** | Async `[vk-validation]` manufactures a false-clean gate | **FOLDED** | §Decision 9b — migration **withdrawn**, channel stays synchronous; E12 conflict resolved explicitly; only the allocation is removed |
| **M9** | Consumer writes land on the producer's line | **FOLDED** | §Data structures `Lane` — third partition for statistics + `owner`; new `offset_of` assert |
| **M10** | Claim scan CASes every occupied lane | **FOLDED** | §Algorithms B — `load`-then-CAS + spread start index |
| **M11** | `Once` still does a per-frame RMW | **FOLDED** | §Decision 8 — steady state is a pure `Relaxed` load, no store; suppressed count for `Once` explicitly not reported |
| **M12** | `RATE[code]` indexes 512 with numbers up to 9101 | **FOLDED** | §Decision 8 / §`LogSite` — dense `code_idx` carried in the site |
| **M13** | `LogRing`'s `Box<[u8]>` is a heap side-store | **FOLDED** | §Data structures — `VmColumn`-backed, engine storage |
| **M14** | `LogFilter` is a divergent mirror with a `dirty` bool | **FOLDED** | §Decision 14 — `LogFilter` deleted; `CEILINGS` is the single owner |
| **M15** | Target IDs are an honour system | **FOLDED** | §Decision 15 — central `targets!` table, compile-time uniqueness for 0..=95; check 7; `TypeIntern` non-applicability recorded |
| **M16** | `flush()` stalls 2 s with no consumer; `join` has no timeout | **FOLDED** | §Decision 12 / §Algorithms D — `SINK_STATE` short-circuit returns `NoConsumer`; shutdown spins bounded then **detaches**; `E0108` |
| **M17** | `LogHandle: !Send` conflicts with `LogPlugin`; nothing owns it | **FOLDED** | §Decision 12 — handle deleted; `boot`/`shutdown` free functions over statics |
| **M18** | Zero-alloc gate cannot cover the shipping config | **FOLDED** | §Metrics test 3 — per-thread const-init counting allocator covers `Thread` mode; process-global variant kept for `Manual` with its limitation stated |
| **M19** | No sustained-rate number; sizing deferred | **FOLDED** | §Decision 10 — design number stated (≥ 500 K rec·s⁻¹ aggregate), adaptive park, `trace!`-in-a-loop declared lossy; L3 gate `sink_sustained_rate` must find the knee |
| **M20** | No perturbation control for the logger itself | **FOLDED** | §Metrics gate P1 (L5-gate) — ABBA + interleaved zero control, `NOT RESOLVED` inside the floor |
| **M21** | No genuinely-off configuration; `.bss` claim ungated | **FOLDED** | §Decision 3 — `LANE_ARRAY_LEN = 0` when `GLOBAL_CEILING == Off`, no thread, no hook; gates G2 (off build) and G3 (section check); "on" floor stated verbatim |
| **M22** | Integration table omits ~40 production sites | **FOLDED** | §Integration — machine-generated `PRINT-CENSUS.md` ledger; measured 179/36 classified into five dispositions; migration split L6/L7/L8a/L8b/L8c; per-crate allows, not per-site |
| **M23** | `disallowed-macros` claimed without a liveness proof | **FOLDED** | §Integration Enforcement — in-repo tidy test is primary; clippy is secondary and only after a shown-red canary; `clippy.toml:21-25`'s silent-ignore finding cited; the lint's blind spots named |
| **M24** | `report!` and the sink share one fd unsynchronised | **FOLDED** | §Decision 9/9b — both take `OUT_LOCK`; console sink → stderr; test 16 runs the **concurrent** state |
| **M25** | L7 control is one this machine cannot show | **FOLDED** | §L7-gate — forced hazard replaced by an ordinary validation error (`mip_levels: 12`) with the baseline-19 accounted; `E2101` made a two-sided gate |
| **M26** | 129th-thread loss is designed in; test 5 one-sided | **FOLDED** | §Decision 5 / E6 — synchronous fallback for `Warn`/`Error` on claim failure; test 8 asserts the fallback delivered and states reclaim is eventual |
| **N27** | `option_env!` evaluation site unspecified | **FOLDED** | §Decision 2 — `GLOBAL_CEILING` is a `const` in `boyko_log`, referenced as `$crate::GLOBAL_CEILING`; never expanded in a caller crate |
| **N28** | `AtomicPtr<u8>` loses the `&str` length | **FOLDED** | §Data structures — `AtomicPtr<TargetInfo>` publishes name+len with one pointer |
| **N29** | `MAX_RECORD_BYTES` reachable from safe code | **FOLDED** | §Data structures / E3 — raised to 2048 **and** checked at runtime in every profile; dropped + `TOO_LARGE` + counted, not a debug panic |
| **N30** | `W0101` has no showable red state | **FOLDED** | §Decision 11 — written down as uncontrolled and listed in `tests/untested_codes.txt` with that reason |
| **N31** | `decode` monomorphisation count is an unmeasured claim | **FOLDED** | §L1-gate G5 — distinct-`decode`-symbol count asserted against an upper bound; the prose claim is removed |
| **N32** | Header duplicates `code`/`level` | **FOLDED** | §Data structures — header shrunk 24 → **20 B packed**; `code`, `level` and `lane` removed (site holds two, the drain knows the third) |

**Refuted: none** (of B1-N32). Every one was either a defect in v1's own pseudocode, a gate that could not fail, or a claim v1 made without a control. Two of these folds were themselves defective and are re-folded below: **M13** (F7 — the fold did not run), **M25** (F2 — the fold installed a second unshowable control). **M11** and **M24** are amended (F10, F13).

---

## Findings disposition (v2 → v3 review, verdict REJECTED)

| # | Finding | Disposition | Where in v3 |
|---|---|---|---|
| **F1** | Test 17 has no red state; its green is this machine's measured normal | **FOLDED** | §Metrics test 17 — positive control (deliberate invalid call, `count ≥ 1`), byte-exact prefix pinned against `vb_bench_query_validation.rs:116-118`, ordering proved by a synchronous marker line, plus an explicit "cannot claim" |
| **F2** | G7's negative side is unshowable here; M25 reintroduced by its own fold | **FOLDED** | §sync-validation confrontation + L7-gate — the two sides become validation-**on** ⇒ fires / validation-**off** (`BOYKO_DISABLE_VALIDATION=1`) ⇒ absent. Both runnable today. "Cannot claim" names the chained case as out of reach |
| **F3** | P1 cannot resolve (FIFO-clamped), and open question 4 reads its silence as proof | **FOLDED** | §Metrics "P1 is re-specified" — headless schedule bench (`bench_bevy_vs_boyko`), not windowed frame time; FIFO confirmed unconditional at `present/swapchain.rs:199`. The inference is **struck** from open question 4; P2's frame-time leg is relabelled drift/leak-only |
| **F4** | G2's two non-size legs have no mechanism; the size leg is a const tautology | **FOLDED** | §Decision 3 — a three-row mechanism table: OS thread count (toolhelp / `/proc/self/task`) **with its own rising-count control**, behavioural panic-hook probe, and the size leg kept but annotated as env-plumbing proof only |
| **F5** | Registry check 3 satisfiable by a doc comment; checks 3 and 6 need opposite walkers | **FOLDED** | §Decision 6 — ONE walker, THREE streams (CODE / LIT / TEXT); each check names the streams it consumes. Measured corpus: 18 doc-comment `B1802` sites in `app.rs` (**not 28** — count corrected, finding stands), plus 6 in `schedule_builder.rs` |
| **F6** | `free` UNDERFLOWS in Algorithms A5; the producer overruns live ring bytes | **FOLDED (correctness bug)** | §Decision 5 + §Algorithms A6 — `avail = CAPACITY − used` (inductive, cannot underflow) and `budget = avail.saturating_sub(ERROR_RESERVE)`. **Gate G17 reds on v2's exact code**; a proptest covers the raw integers |
| **F7** | `LogRing`'s `VmColumn<LogLine>` assert-panics at construction; M13's fold does not run | **FOLDED (correctness bug)** | §Data structures — `LogLine` is **16 B**, `#[derive(Clone, Copy)]`, with `const _: () = assert!(COMMIT_GRANULE % size_of::<LogLine>() == 0)`. Verified against `vm_column.rs:144-149` and `constants.rs:7`. `TargetStat` gets the same pin (32 B). `pub(crate)` visibility is a non-issue: `LogRing` lives inside `boyko_ecs` and the field is private |
| **F8** | `OUT_LOCK` is an unbounded spin with no poison story; Invariant 6 violated | **FOLDED** | §Decision 9c — format-before-lock, re-entrancy detection via `OUT_OWNER`, 50 ms bounded acquire then **steal**, RAII release on unwind. Gate G18 two-sided. E25 states the steal |
| **F9** | `OUT_LOCK` cannot be registered in `HOT-PATH-EXCEPTIONS.md`; doing so reds CI | **FOLDED** | §Invariant 1 — verified against `scripts/check_hotpath_exceptions.py:15-19,51,337-341`. **No row is added.** Open question 7 is rewritten from "confirm the entry" to "confirm the steal trade" |
| **F10** | `Once` suppression is invisible to the census too; contradicts the Goal | **FOLDED** | §Goal bullet 3 + §Decision 8 — three quantities kept separate; the census prints `suppressed=UNCOUNTED(by policy)` so the *absence of a count is itself printed*; `OnceCounted` offered with its RMW cost stated at the declaration site |
| **F11** | Rate policy is code-scoped while one code covers N sites; `W2102` silences two of three | **FOLDED** | §Decision 8 — `Once`/`OnceCounted` become **per-SITE** (a macro-generated `static FIRED` beside the `LogSite`). Cheaper too: a private line instead of a shared `RATE` line. Test 31; behaviour-ledger rows for `W2102`/`W2202` updated |
| **F12** | Decision 9b's justification for touching `debug.rs` is fictitious and its edit changes bytes | **FOLDED** | §Decision 9b — the edit is **withdrawn entirely**; the site is untouched and allowlisted with a reason. `Cow::Borrowed` (no allocation), byte change only in the invalid-UTF-8 case, and the `stderr()`-lock regression are all named |
| **F13** | M24's "verified" claim misdescribes its own consumer | **FOLDED** | §Invariant 2 + §Compatibility + test 16 — `vg_occ_split_timing.rs:1115-1117` reads `stdout`/`stderr` as separate buffers; the merged consumer is `golden.ps1:196-202` via `cmd /c`. The real hazard is intra-stdout (F17) |
| **F14** | Machine-consumer inventory incomplete; a third gate's entire input uncited | **FOLDED** | §Invariant 2 — measured inventory: **31 files** reference `[vk-validation]`, **16** reference `VB-P1d`. `vb_bench_query_validation.rs:116-118`'s byte-exact constant (trailing space) is now pinned by test 17 |
| **F15** | `TargetId(pub u16)` vs `MAX_TARGETS` with a debug-only bound | **FOLDED** | §Decision 15 + §Data structures — private field, closed constructor set, `get_unchecked` sound with a SAFETY clause naming it. `INVALID` deleted in favour of `Option<TargetId>`; E19 rewritten; the `debug_assert` removed as vacuous |
| **F16** | Principle 0 exception neither named nor argued | **FOLDED** | §Invariant 7 — claimed in writing on dependency inversion, with the cost (no `Query`, no change detection) stated and `CONTROL_EPOCH` named as the mitigation |
| **F17** | Buffered-stdout race between `report!` and the surviving `println!` sites | **FOLDED** | §Decision 9 — `report!` writes **through `stdout()`'s own `LineWriter`**, not a raw handle; cost (one memcpy + one flush) stated. Test 16's red state is the raw-handle variant |
| **F18** | Census arithmetic: 83 vs the ledger's 98, over a corpus containing non-sites | **FOLDED** | §Goal + §Integration ledger — 179/36 reproduced by one command; 98 named as the occurrence remainder; the **walker's site count** named as the migration denominator so comment mentions can never be driven into the allowlist |
| **F19** | G1 is scheduled at L0-gate but depends on L1 | **FOLDED** | §Implementation plan — G1 moves to **L1-gate**; L0-gate keeps G4 and G2, both of which have real red states at L0 |
| **F20** | L2-gate reds (or goes vacuous) on the grandfathered corpus | **FOLDED** | §Decision 6 — registry rows carry `Live` / `Pending(rung)`; check 3 (Live ⇒ ≥1 emitter), check 3b (Pending ⇒ 0 emitters), check 3c (`Pending == 0`, armed at L8c). L2 commits alone; a `Pending` row cannot rot silently |
| **F21** | `LANE_ARRAY_LEN = 0` vs `LANES[i]` indexing in Algorithms B | **FOLDED** | §Decision 3 + §Algorithms B — the claim scan is written over `LANES.iter()`; zero-length is zero iterations |
| **F22** | `dsp!`'s described form does not borrow-check | **FOLDED** | §Decision 13 — the `DspBuf<N>` by-value-temporary form is written out, with the end-of-statement lifetime argument and the 256-byte cost in the trade-off |
| **F23** | `Lane`'s SAFETY block does not cover `read_cached` / `write_cached` | **FOLDED** | §Data structures SAFETY clauses **1e** and **1f** — single-role ownership plus the staleness argument in both directions |
| **F24** | `W0102` is unbudgeted: ~16 000 sink-generated records/s during a drop storm | **FOLDED** | §Decision 5 + §Algorithms C — **one aggregated `W0102` per drain** (125/s) carrying `lanes_affected`/`records`/`bytes` and, since v4, the `LossClass` breakdown in place of v3's `SATURATED` flag (S8); per-lane detail moves to the polled census |
| **F25** | `STAGE_BYTES`'s backing store unspecified | **FOLDED** | §Data structures — `STAGE`, `SITE_DICT`, `SINK_OUT` are `.bss` statics, counted in Decision 3's budget matrix, with the "no `Vec`/`Box` in a *signature*" narrowness called out |
| **F26** | Zero-alloc test 3 proves only the steady state (TLS dtor registration) | **FOLDED** | §Metrics test 3 — three legs (steady 0, first-emit ≤ 1 with the number recorded, monotonicity over 1000 emits) and a new perf-table row |

### Refuted, with the evidence

| # | Claim | Refutation |
|---|---|---|
| **F5 (count)** | "28 occurrences of `boyko-B1802`, every one inside a `/// # Panics` doc comment" | **Partially refuted.** `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences: 18 in doc comments, 1 panic-message string (`:867`), 5 `#[should_panic(expected=)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). The *finding* is correct and folded; the *count* is not, and the fix is specified against the measured shape (the `#[cfg(test)]` region also has to be stripped, which the review's framing did not surface) |
| **F12 (part)** | "`eprintln!` currently takes `stderr()`'s own lock, so the replacement … is a regression against M24's own concern" | **Accepted as to `debug.rs`** — and the conclusion is stronger than the review's: the whole edit is withdrawn, so the concern cannot arise. But the *general* claim that `stderr()`'s lock made v2 safe is not why `report!` needed fixing: `report!` writes **stdout**, where the hazard is the `LineWriter`, not the lock (F17). The two are separate defects and are fixed separately |
| **F16 (framing)** | "`boyko_log`'s statics are almost certainly correct but Principle 0's named exceptions do not cover them" | **Accepted, and the exception is claimed** — but not as a formality. The argument is *load-bearing*: it is the same argument that forbids `CONTROL` from being an ECS column (§Refused), and it is what makes the crate usable from a driver callback and a panic hook. Recording it as a bare exception would lose that |

**One finding is deliberately NOT folded as written:** the review's open question 5 asks "`LogLine` must be `Copy` and its size must divide 64 KiB. Which size — 8 or 16?" **16**, with the fields listed in §Data structures. 8 bytes cannot carry `start` + `len` + `code` + `level` + `target` without dropping the sequence number that Decision 26's reader cursor needs.

---

## Findings disposition (v3 → v4 review, verdict REJECTED)

| # | Finding | Disposition | Where in v4 |
|---|---|---|---|
| **B1** | `LogRing`/`LogCensus` cannot be `Resource`s — `VmColumn` is `!Send + !Sync` | **FOLDED (correctness bug)** | §Data structures, "B1" block — verified against `vm_column.rs:70` and `resource.rs:42`, both re-read this session. A SEND10-shaped `unsafe impl` with a **named holder set**: `ResMut` only in `log_drain_system`, `Res` readers via the scheduler's exclusivity, and — the load-bearing clause — **the sink thread never touches either type**, because B2 gives it `ECS_HANDOFF` instead. Quotes `vm_column.rs:73-77`'s own invariant list for the columns, and notes that `LogPlugin::build` materialises before the schedule runs so the write-once `base` never races. Pinned by `const _ { assert_send_sync::<LogRing>() … }` — F7's treatment applied to `Send`/`Sync` |
| **B2** | The sink→ECS "handoff ring" is referenced three times and defined nowhere | **FOLDED** | §Decision 26 + §Data structures + §Algorithms C — `ECS_HANDOFF`, a first-class `HandoffRing` with **the same shape and wrap rule as `LogLane`** (no new protocol), plus type, capacity (256 KiB / 64 KiB), ordering rows in §Multithreading, a `.bss` budget row in Decision 3, overflow accounting (`LossClass::Sink`, `W0117`, `lossy`), a four-clause SAFETY block, and a presence rule (absent when `ecs_ring` is off). G15 gains leg (c) |
| **B3** | G17 leg (c) cannot go red — the F6 overrun is intra-lane by construction | **FOLDED** | §Gates G17 — the neighbouring-canary claim is **deleted** (and relocated to test 6's wrap proptest, where an off-by-one *can* cross a lane). Leg (c) becomes "pre-seeded **undrained** records are byte-unmodified after the refused emit", **at a second, explicitly named fill level** (`used > CAPACITY − need`), because at the reserve-boundary fill the broken arithmetic writes into genuinely free space and leg (c) would be vacuous there too. The arithmetic for both fills is worked out in the gate row |
| **B4** | G4 leg (d) measures argument evaluations while asserting a delivered count, and needs L12's mechanism at L0 | **FOLDED** | §L0-gate (leg deleted) + §Gates **G10e** at L12-gate (leg re-created with both numbers: **1000 evaluations AND 500 delivered**) + §Algorithms A (**LANE moved ahead of SAMPLE**, so no lane-indexed state is touched before the lane exists, and E6's unlaned thread skips sampling rather than indexing a row it has no claim to). RATE keeps its position, since it is code- and site-indexed, so a suppressed record still costs no lane claim |
| **B5** | The crash-drain CAS treats `Manual` as quiescent; it is not | **FOLDED (correctness bug)** | §Decision 24 — `DRAIN_OWNER`, an `AtomicU64` role token CAS'd identically by all **four** consumers (sink thread, `drain()`, `Scheduled`, crash). `SINK_STATE` loses its exclusivity job and `CrashDraining`. `drain()` returns `DrainResult::Busy` rather than asserting, because a second manual caller is a user error and a `debug_assert` is silent in release. §Gates **G14 gains leg (c)** — panic with a manual drain held open by a barrier — with the red state being v3's exact CAS. E26/E27 added; the `DRAIN_OWNER` CAS is **release-live** |
| **B6** | Registry check 4 cannot be green at L2 — its corpus names ~25 unregistered codes | **FOLDED, with the measurement redone** | §Decision 6 — TEXT becomes an **explicit directory list excluding `docs/archive/**`**; a `Historical` row status is defined for any future re-inclusion; `docs/diagnostics/B9004.md` and `B9005.md` are named as L2 line items (both exist in source and in **no** document); the block map's "occupied today" column is seeded from the measured 9 distinct source codes; `92xx` is reserved at L2 with 17 `Pending` rows and its own measured free-band note. **Measured here**: 75 occurrences / 13 files, `docs/archive/**` **29** (not 27), and a case the review and the addendum both missed — code **W9003 at `docs/archive/PHASE-15-PLAN.md:471`**, which is *also* this document's own check-4 red-state example, so v3 would have red **on itself**. Every prefixed literal in v4 names a code this plan registers; unregistered codes are named bare |
| **B7** | The walker's `#[cfg(test)]` rule cannot exclude the files the ledger says it excludes | **FOLDED** | §Decision 6 — the rule is **cross-file** and specified without a Rust parser: a pre-pass collects `#[cfg(test)]` + optional `#[path]` + `mod NAME;` declarations and marks the resolved file test-only. Verified per file: `compute/tests.rs` 16 within-file; `brick/tests.rs` gated at `brick.rs:1829-1830`; `colored_tests.rs` gated at **`colored.rs:3198-3200` through a `#[path]`** — which the review noted was ungated but did not locate, and which is why the rule must follow `#[path]` too. `#[cfg(any(test, …))]` is treated as test-only **and listed by name** in the walker's report. The `179 − 58 − 23` arithmetic is re-derived against the rule that will run |
| **B8** | `shipping-min`'s crash file structurally contains the session's beginning | **FOLDED** | §Decision 10 (new `SinkMode::Scheduled`, with **both** of v3's rejection grounds addressed — one answered by `DRAIN_OWNER`, the other **conceded and written down as the profile's cost**), §Decision 25 (`ShippingMin` uses it), §Decision 5 (why the answer is not overrun-oldest), C4, E22. **The retained window is restated in RECORDS**: ≈ 13 100 across 32 lanes, ≈ 410 per lane — and under `Scheduled` that ceiling is never approached, so the crash file holds records adjacent to the crash |
| **B9** | The sync route, the exhaustion fallback and E22's mitigation all write a stderr a shipped title does not have | **FOLDED, with one premise corrected** | §Decision 9c "the durable fan-out" — `write_oracle_line` writes **every** configured synchronous destination, including the boot-opened crash file; G18 gains leg (c). Decision 20's cost is restated: **~200 ns is uncontended and console-only**, the contended bound is `OUT_LOCK`'s 50 ms steal deadline, and "durable-on-write" now means a `write_all`, with `fsync` opt-in (`sync_durable`) at ~0.1-10 ms. **Premise corrected**: `grep -rn windows_subsystem crates src` returns nothing, so stderr is a valid handle on this tree *today*; the invalid-handle case is a future shipping configuration. The **durability** defect was real in every profile including `dev`, which is the half that carries the fix |
| **B10** | `LogPod`'s blanket encode copies padding; the sink materialises `&[u8]` over uninitialised bytes | **FOLDED (correctness bug)** | §Decision 19b — the blanket `copy_nonoverlapping` is **deleted**. The trait requires `unsafe fn encode_pod(&self, dst: *mut u8)`; the derive generates it **field-by-field through `LogValue`** (which it already required), rejects dynamic-length fields so the sum stays a `const`, and emits `const _: () = assert!(POD_LEN == Σ field lengths)`. `POD_LEN` is no longer `size_of::<Self>()`. **G9b's subject changes** to the padded-encode red, and the `debug_assert!` list loses `POD_LEN == size_of::<Self>()` — the assertion that made the UB look checked |
| **M1** | The census line that fixes F10 is uncomputable after F11 moved the latch per-site | **FOLDED** | §Decision 8 point 2 + §Data structures `OnceSite` — an intrusive, insert-only `ONCE_SITES` list whose **nodes are the per-site statics the macro already expands**; pushed by a `#[cold]` CAS on the site's single fire, so nothing is added to the steady-state path. The census prints **one row per fired site** (three rows for `W2102`, which is the F11 case), and a never-fired site is **absent**, which is itself the datum. **`RateSlot::fired` is deleted** as dead. Test 31 extended |
| **M2** | The session-scale audit omits `seq_lo` and every `BinarySink` quantity | **FOLDED** | §Decision 21 — six ✚-marked rows added: `seq_lo` **with its reconstruction rule** (`seq = ring.seq − (ring.seq_lo ⊖ line.seq_lo)`, unambiguous because the ring holds ≪ 2³¹ lines, so the high half is never stored and never needed), `tsc_delta` **with the anchor cadence its 1.4 s span forces**, `site_id` against the 4096-entry dict (with `W0116` + inline site records on a full table), `len`/`flags`, file offsets and the rotation counter, and `clock_epoch_lo`. Decision 22 states that the widths are pinned **here**, not deferred to `docs/LOG-BINARY-FORMAT.md`. Test 20 and a property family extended |
| **M3** | Downstream code-index exhaustion has no defined behaviour | **FOLDED** | §Decision 19 — `CODE_IDX_EXHAUSTED`, a reserved sentinel that is **never an aliased index**; the record is **still delivered** with `Every` semantics; `boyko-E0115` once; `LogStats.codes_unindexed` thereafter. **G9 gains an exhaustion leg** whose red state is a modulo-wrapping mint that shares a `RateSlot`. E30 added |
| **M4** | The census's own showcase row is permanently `UNPROVEN` by construction | **FOLDED** | §sync-validation confrontation — **the `vk-validation` census row and target are deleted**, with the reason written where the row was: Decision 9b guarantees no record can ever reach that target, so the row could never move and invited the opposite misreading. The census is illustrated by a target records actually reach; `E2101` + G7 remain the liveness claim. Test 17 leg (c) loses its census clause. The review's alternative (count messenger callbacks) is rejected in writing, because it requires editing the byte-frozen `debug.rs:114` |

### Refuted, with the evidence

| # | Claim | Refutation |
|---|---|---|
| **B6 (archive count)** | "`docs/archive/*` (10 files) — 27" | **Refuted as to the number.** Re-measured this session with `grep -roE 'boyko-[BEW][0-9]{4}' docs --include='*.md'`: `docs/archive/**` contributes **29** occurrences across 10 files, and the four-way split is 41 / 29 / 3 / 2 = **75**, which is the addendum's own stated total (its 41 + 27 + 3 + 2 sums to 73). The **finding is correct and folded**; the count is not, and the fix is written against the measured composition |
| **B6 (missed code)** | The archive-only dead codes are "`B9000` and `B9003`" | **Incomplete, and the omission matters.** The archive's distinct set is `B0002 B1801 B9000 B9001 B9002 B9003 B9101 W1501 W9003`, so the codes with **no emitter and no current doc** are `B9000`, `B9003` **and `W9003`**. `W9003` is the one that bites: v3's check-4 row used its **prefixed literal** as the *illustrative red state*, and check 4 scans this file — so v3's check 4 would have red on this very document, permanently, against a real archive code. Both facts are folded (the corpus excludes `docs/archive/**`, and unregistered codes are named bare in this document); neither the review nor the addendum had the second |
| **B9 (premise)** | "a shipped windowed Win32 title does not have [stderr]" / "in any windowed Win32 title, where stderr is an invalid handle" | **Refuted as to the tree, accepted as to the design.** `grep -rn windows_subsystem crates src` returns **nothing**: every binary here is a console-subsystem binary and stderr is a valid handle today, so the invalid-handle failure is a *future* configuration rather than a present one. The finding survives the correction with room to spare on its other leg — stderr is neither the log file nor `fsync`ed, so "durable-on-write" was false in **every** profile including `dev`, and `shipping`/`shipping-min` configure no console sink at all, so the bytes go nowhere collected. The fix is unchanged; only the argument for it is |
| **S5 (`E0110`)** | The seam record specifies "`E0110` on a ninth [`PRE_FLUSH` registration]" | **Refuted as to the number.** `W0110` is already `OUT_LOCK`'s steal code. `DIAGNOSTICS` is dense with `index == code_idx`, and registry check 1 asserts numbers **strictly increasing**, which is also a `const _: () = assert!` — two rows numbered 110 would not compile, whatever their class letters. The mechanism is adopted verbatim; the code is **`E0118`**, the next free slot in the `01xx` band. Recorded in §Seam disposition so the divergence is not read as a transcription error |

**Not refused, and worth saying so:** every one of B1-B10 and M1-M4 is folded. Four of them (B1, B5, B8, B10) are correctness bugs of the same family this campaign keeps finding — a proof that holds for the case the author had in mind and not for the case the code admits. Three (B3, B4, M4) are gates that could not go red, found **inside** the gates written to stop gates that cannot go red.

---

## Seam disposition (S1-S12)

Against `docs/DIAGNOSTICS-SUBSTRATE-PLAN.md`'s decision record. **These decisions are made, not re-litigated**; the column that matters is where each one lands in this document.

| # | Seam decision | This plan's disposition | Where in v4 |
|---|---|---|---|
| **S1** | The profiler owns the measurement channel; `report!` is deleted | **ADOPTED, in full, including the cost** | §Decision 9 rewritten around "nothing in the engine writes stdout"; `report!` struck from §Public API, `sync_out.rs`'s file list, §Multithreading's `OUT_LOCK` row, the ledger and the behaviour table; **mandatory test 16 deleted** (number not reused); L8b's measurement rows → **0**; the denominator ≤ 98 → **≤ 78**; open question 1 struck. **`OUT_LOCK` survives** with its seven remaining callers enumerated in Decision 9c, so Decision 9c and G18 stand whole. The six stdout consumers are named in Decision 9 and their migration is the profiling plan's rung 7, which L8b now waits on |
| **S2** | `boyko_diag` is the new bottom; `boyko_utils` keeps zero deps | **ADOPTED** | §Affected subsystems rewritten with the acyclicity proof; **"`boyko_utils` … depend on it" struck**; Decision 15's `TypeIntern` note keeps its surviving reason (`ID` must be a `const`) and loses the false one; test 33 asserts the manifests |
| **S3** | One lane registry in `boyko_diag`, deterministic worker-anchored topology | **ADOPTED, and it deletes five things from this crate** | §Decision 4 rewritten; `MAX_LANES`, the `hash(thread_id)` scan, the `owner` field, `MY_LANE` and the `Drop`-guard TLS all deleted; §Algorithms B is now a TLS read; `LogLane`'s SAFETY clause 1 re-argued against the substrate's single-writer sources; Decision 3's matrix at 80/32 lanes; **the "≤ 1 allocation on first emit" row becomes `0`** and test 3 leg (b) becomes `== 0`; the convoy and leak costs are stated and bounded (14 CAS attempts; 224 KiB, counted as `lanes_leaked`); test 8 gains S3's three reds **including the join red** |
| **S4** | One clock in `boyko_diag`; `W0101` deleted; `RecordHeader` gains `clock_epoch` | **ADOPTED, and the L3 choice is made here** | §Decision 11 rewritten; `tsc.rs` deleted; `W0101` struck from Decision 11, the gates' "no control possible" paragraph and `untested_codes.txt`. **The header does not grow**: `clock_epoch_lo: u8` spends v3's `_pad`, so `HEADER_BYTES == 20` stands (S4 deferred the 4-byte-vs-4-bit choice to L3; Decision 11 takes it, with the one-park-interval argument for why 8 bits suffice). The benefit is recorded as **agreement, not speed** — ~one `cpuid`, not 20 ms. Test 32 added |
| **S5** | `boyko_app` owns the lifecycle; `PRE_FLUSH`; `sink_can_accept()`; deferred diagnostics | **ADOPTED, with ONE numeric divergence** | §Decision 12 gains the whole lifecycle block (order with **`flush_gpu` ahead of `flush`**, the `PRE_FLUSH` array and its registrant contract, `sink_can_accept()`, and `boyko_diag::raise` deferral) plus S5's four reds at L3-gate; §Performance targets gains `log_disabled_warn ≤ 4 ns`; E1/E2/E16/E29 updated. **Divergence**: the ninth-registration code is **`E0118`, not `E0110`** — see §Refuted |
| **S6** | Reserve `92xx` at L2 as 17 `Pending` rows; check 2 narrowed to `Live` | **ADOPTED, and extended by the orchestrator addendum** | §Decision 6 — the block map gains a measured `92xx` row; check 2 is `Live`-only; the `Pending`/`Live`/**`Historical`** statuses are defined; L2's line items are itemised. The addendum's three facts (archive exclusion, `B9004`/`B9005` doc debt, measured `9xxx` occupancy) are folded, with the archive count corrected to 29 and `W9003` added |
| **S7** | One rule each for stdout, stderr and files; the profiler's report has no console form | **ADOPTED** | §Invariant 2 gains the two-line rule; Decision 9 states it as the replacement for `report!`; Decision 9b gains the **shared-handle** clause (line integrity, not ordering, is what the gate consumes); the stderr line-integrity red lands at L3-gate as test 16's replacement |
| **S8** | One loss vocabulary; **accumulate in `u64`, never saturate**; one `DiagCensus` | **ADOPTED — it reverses this plan's own X4** | §Decision 5 (counters), §Decision 17 (status table, `SATURATED` struck), §Decision 21 (row rewritten), §Data structures (`LossCell`s replace the `AtomicU32` pair), §Gates **G11's subject replaced** by fold exactness under a live producer, E18 rewritten, `DiagCensus` named in §Data structures and C-conflicts. X4's rejection of `u64` ("an 8-byte RMW is more expensive") does not survive: `lock xadd` costs the same at 4 and 8 bytes and the lane-owned cell needs no RMW at all |
| **S9** | One axis, `BOYKO_PROFILE`, owned by `boyko_diag/build.rs`; `LogRuntimePreset` | **ADOPTED** | §Decision 25 rewritten as two tables (compile axis / runtime preset), **the `GLOBAL_CEILING` column removed** from the preset, the preset renamed, five profiles, **`crates/boyko_log/build.rs` not created**; Decision 2 re-sourced to `boyko_diag/build.rs` with N27's property preserved verbatim; Decision 3's off-switch is `BOYKO_PROFILE=off`; G16 grows to four legs; the 5-legs/4-net-new CI figure and the `target/` disk rule are recorded |
| **S10** | Both perturbation gates run in the both-present configuration; one final joint baseline | **ADOPTED, with one arithmetic correction** | §Performance targets (the working-set row carries **both** figures, 4 in isolation and 7-8 jointly, and the joint claim appears at the number rather than only in a table), §Metrics P1 becomes a **2×2**, every bench baseline carries `config_tag`, revert clauses record `UNPROVEN` until **J2**, and J2 is a rung. **Correction**: the seam record's joint `dev` total of 9.33 MiB predates `ECS_HANDOFF`, which B2 adds in this revision; the figure is **≈ 9.58 MiB**, derived in Decision 3. Retail is unchanged at ≈ 1.95 MiB |
| **S11** | Vocabulary renames | **ADOPTED** | `Lane` → `LogLane`; `MY_LANE`/`MAX_LANES`/claim scan → `boyko_diag::lane()`/`LANE_COUNT`; `LogConfig` profile → `LogRuntimePreset`; `CONTROL_EPOCH` → `CONTROL_EPOCH_CTR`/`control_epoch()`, `EPOCH` record → `frame_epoch`, `FLUSH_REQ`'s epoch → `FLUSH_SEQ`; "windowed frame time" → "**presented** frame time" and `window` reserved for the statistics horizon; own `SessionId` mint → `boyko_diag::SessionId`; `report!` deleted (S1); `W0101` deleted (S4) |
| **S12** | One never-freed-storage policy: compile-time extent ⇒ `.bss`, run-time extent ⇒ `VmReservation` | **ADOPTED — and it turns Invariant 7 from a plea into a rule** | §Invariant 7 rewritten: the Principle-0 argument is no longer "these statics are an exception" but "the extent of these tables is a compile-time const, therefore `.bss`", with the reason the boundary is *forced* (`VmReservation` is `pub(crate)` with a `libc` arm, so a std-only zero-dep leaf cannot host it without minting a second per-OS backing). G3 uses **`boyko_diag::section_report`**, one implementation for both plans. `boyko_demo`'s third-party `log = "0.4"` deletion and the manifest tidy check land at L8b (test 33) |

**Open, and NOT this document's to close** — both are recorded in §Open questions as owner calls: the **1.95 MiB joint retail** figure against the profiling plan's "≤ 1 MiB retail" headline (VALUES), and whether `shipping-min` should also disable the profiler's telemetry stream, which still writes synchronously on the dispatcher in that profile (SCOPE).

---

## Scope-extension disposition (games as a first-class audience)

### What the extension CHANGED in the engine design, and the argument for each

| # | v2 element | Change | Argument |
|---|---|---|---|
| **X1** | `CEILINGS: [AtomicU8; 256]`, one byte = one level | Renamed `CONTROL`; the byte is packed `[0..2] level ‖ [3..6] sample shift ‖ [7] sync route` | Three runtime knobs delivered in **the register already loaded** — no second array, no second cache line, one extra `and`. Decision 14's single-owner property is preserved: still one byte, one authority, no mirror. `.bss`-zero still means fully off. **Gated**: `log_disabled_runtime` must stay NOT RESOLVED against the v2 shape (G10d), else the packing is reverted |
| **X2** | Downstream target band 96..=255 | Re-cut: source 96..=223, **dynamic 224..=255** | The dynamic band is the concrete answer to "declared from data / a mod / a script". 32 slots is a deliberate cap (open question 8). Decision 15's *mechanism* is unchanged |
| **X3** | `RatePolicy::EveryN(u16)`, arbitrary `n` | `n` must be a power of two (`const _` assert); `count & (n-1)` replaces `count % n` | v2's form mis-samples across the `u32` wrap (~12 h at 100 K·s⁻¹) — invisible in a 300-frame bench, wrong in a session. The fix is *also* cheaper. Strictly better on both axes |
| **X4** | `dropped` / `dropped_bytes`: plain `fetch_add` | v3: **saturating `u32`** + a `SATURATED` census token. **v4 REVERSES this to `u64` accumulation** (S8) | v3's premise — "with no drain running the counter wraps in ~65 s and then reports a small, credible, wrong number" — was right, and its remedy was wrong. It rejected `AtomicU64` on "an 8-byte RMW is more expensive"; on x86-64 `lock xadd` costs the same at 4 and 8 bytes, **and the lane-owned cell needs no RMW at all** (single writer). With `u64` there is no ceiling in ~8 800 years, so the `SATURATED` token — which a reader could never *compare* — stops existing. The `.bss` cost is 8 bytes per class per lane. **This is the one v3 element v4 undoes rather than extends**, and it is undone because its stated reason did not survive being checked |
| **X5** | `LogSite` fields | `+ fields: &'static [&'static str]`, `+ prefix` | Structured/telemetry output needs names, and a game needs its own code prefix. `LogSite` is `&'static`, cold, and never touched on the emission path — free everywhere it matters |
| **X6** | `LogLane` line 2 | `+ sampled_out` (now `AtomicU64`, alongside the `LossCell` array) | A sampled-out record is **not** a loss and therefore **not** a `LossClass`; counting it into `dropped` would corrupt the drop count in the other direction, and the `emitted == drained + dropped + sampled_out` property depends on the separation |
| **X7** | `FileSink { path, max_bytes }`, stop-at-cap | `+ Rotation { max_bytes, keep }`; **`Rotation::NONE` remains the engine default** | An hours-long session needs rotation; a bench must not silently discard its own beginning. Both, selected by profile. Every rotated file re-emits anchor + dictionary so it decodes standalone |
| **X8** | Sinks "boot-published, never mutated after boot" | Kind stays boot-fixed; **state / filter / floor become runtime byte stores**; open/close go through a 16-entry `SINK_REQ` under `OUT_LOCK` | A game toggles capture from a console with no restart. All I/O stays on the sink thread (G13b proves zero allocations on the requesting thread). A channel was rejected: an allocation and usually a `Mutex` |
| **X9** | `SINK_STATE ∈ {NotBooted, Running, Manual, Exited}` | v3: `+ Exiting, CrashDraining`. **v4: `+ Exiting, Scheduled`; `CrashDraining` deleted** (B5, B8) | v3 gave `SINK_STATE` two jobs — lifecycle **and** consumer exclusivity — and the second one was wrong about `Manual`. v4 splits them: `DRAIN_OWNER` is the exclusivity token (so `CrashDraining` has nothing left to mean), and `Scheduled` is the lifecycle state for the profile whose consumer is the schedule |
| **X10** | Census statuses `Measured / Unproven / UnprovenLossy` | `+ UnprovenSampled`, `+ UnprovenUnsunk` (v3 also added `dropped=SATURATED`; **v4 strikes it**, S8) | The second audience creates new ways to build a vacuous gate: sample a target and read the count as a total; enable a target no sink accepts and read the silence as clean. Each gets a status; `unsunk` also gets `W0111`. The saturation status is gone because the state is gone. **And one v3 status *instance* is deleted**: the `vk-validation` row, which could never move (M4) — a status vocabulary is only worth having if every row using it can change |
| **X11** | `CensusPolicy` implicit (`OnFlush`) | Explicit `OnFlush` (dev default, unchanged) `/ OnShutdown / Interval(secs)` | `OnFlush` in a game that flushes per frame is a per-frame census line. The engine default does not move |
| **X12** | Perf target rows | 6 rows added; **no existing row weakened**; the two measured rows keep their numbers **and gain a gate that can revert the rung threatening them** | The extension is not permitted to buy flexibility with the engine's measured budget. Where a new mechanism touches a measured path, the gate's failure disposition is "revert or feature-gate the extension", never "raise the target" |

### What the extension deliberately did NOT change

The `[vk-validation]` channel (**not edited at all** — v2's one edit is withdrawn too) · Decision 7 (`Warn`/`Error` MUST carry a code — no exception for games, mods or scripts) · Decision 13's structural re-entrancy exclusion (`LogPod::fmt_pod` runs on the sink, asserted by test 24) · Decision 12's core (no handle, bounded waits, detach-not-join) · Decision 1's deferred-format core and the **20 B** packed header · the eight registry checks · Decision 3's off-build · the `.bss`/`Off == 0` regime · the SPSC lane ring and its SAFETY clauses · gates G1-G7 and their limits · every migration-ledger disposition except the measurement rows.

*(Four items on v3's list of "did not change" **did** change in v4, and are listed here so a reader of both versions is not misled: **`report!`** is deleted (S1); **`W0101`** is deleted (S4); the registry's **corpus rules** are re-cut (B6, B7); and Decision 12 gains `PRE_FLUSH` and `sink_can_accept()` (S5). None of the four was touched by the scope extension — they were moved by the seam and by the third-pass review.)*

### Refused, with the reason recorded so it is not re-derived

| Ask | Reason |
|---|---|
| **Network sink inside `boyko_log`** | Blocking, retries, back-pressure — the three properties this crate refuses. `CallbackSink` on the sink thread is the seam; the game owns the policy because only the game knows whether stalling or dropping is worse for it. G13c demonstrates the stall cost rather than leaving it as prose |
| **Cross-process shared-memory ring** | The record carries a process-local `*const LogSite`. Supported instead: per-process files, one `SessionId`, `logdec --merge`, with the clock-agreement bound **printed** |
| **Gameplay decisions on log counters** | Lower bounds under drop, schedule-dependent, non-deterministic across machines ⇒ breaks replay. Display and telemetry are supported (`LogCensus.lossy` is the bit that keeps a UI honest); gameplay counters belong in the game's own components — Principle 0's answer |
| **Data/script-defined *codes*** | A code is a promise of a documented page; a data-defined code cannot have one. Supported instead: one class code per game subsystem, plus dynamic *targets* |
| **`CONTROL` as an ECS column with `EnableTag`** | Dependency cycle · no `World` at boot, in a panic hook or in a driver callback · ~10-30 ns per emit against a 1-instruction budget. The rule's substance (capability structural, state a bit) is applied at the layer that can afford it, and the exact cost of the refusal (no `Query`, no change detection) is stated with `CONTROL_EPOCH` as the mitigation |
| **Per-entity log storage in `ComponentPool`** | Logging is not per-entity data; `UnitId`'s two-level addressing buys nothing for a byte ring |
| **`Box<dyn Sink>` plugins** | `extern "C" fn(&FormattedRecord, *mut ())` + ctx crosses a dylib boundary with no vtable and no allocation |
| **A second sink thread for the binary sink** | Two consumers on one lane set is what the `LogLane` SAFETY block forbids — and after B5 that is enforced by a `DRAIN_OWNER` CAS a second thread would simply lose. Sinks fan out **inside** one drain, so text + binary + crash cost one pass |
| **Reaching the file sink's handle from `write_oracle_line`** | That handle is owned by the consumer role; writing it synchronously from an arbitrary thread is a second consumer of the sink's own state. The durable fan-out therefore targets the **crash** handle, which is opened at boot and appended to under `OUT_LOCK` — one destination, one lock, no second consumer (B9) |
| **Growing the ring to answer "as much data as possible"** | Enlarging the ring moves the loss point; it does not raise the throughput ceiling, which is `core::fmt` on the sink. The answer is to **not format** (`BinarySink`) — and that claim ships with a revert clause (G12c) rather than as an assertion |

### The one thing this plan says plainly is a bad idea

**Do not make the logger a substitute for a missing source.** This is not a hypothetical: sync-validation is **dead on this machine** — a genuine missed barrier produced 19 messages (the baseline), zero `SYNC-HAZARD`, and a byte-identical golden, twice. A logger is a transport. It changes where a message goes and has no opinion on whether the message exists. Routing a dead channel through a prettier pipe makes the deadness *harder* to see, which is why v1's migration is withdrawn and why the census reports `UNPROVEN`, never `clean`.

The same reasoning bounds the game-facing ask. A logger cannot tell a game why its frame hitched if nothing measures frames; it cannot tell a player's support agent what went wrong in a crash that did not unwind; and it cannot make a sampled capture representative. Each of those is written into a gate's "cannot claim" column rather than left for a reader to discover at the moment they need it to be true.