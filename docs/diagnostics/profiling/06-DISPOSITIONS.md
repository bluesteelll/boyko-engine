# Profiling — dispositions, answers, open questions, and the audit trail

<!-- CONTRACT
exports: profiling/dispositions
assumes: substrate/lane-write-sites
assumes: substrate/loss-fold
assumes: seam/decisions-s1-s12
assumes: seam/free-when-off
assumes: seam/joint-cost
assumes: seam/open-owner-calls
assumes: profiling/goal-and-audiences
assumes: profiling/contrast-api
assumes: profiling/ladder
assumes: profiling/gates
-->

**Carved from** `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §"Answers to the review's open
questions", §"Answers to the scope extension's questions", §"Open questions (remaining)",
§Checklist, §"What rev 3 changed that rev 2 had EARNED", and all three findings-disposition
tables. The **fourth table (B1-B6 / M7-M13) is assembled at the split**; see the repair note
below. Diff against that document until it is retired.

> **This file is allowed to be long. It is the audit trail, and losing a row from it is a silent
> regression.** Every `F`, `X`, `B` and `M` has a row here, including the ones the design
> *refutes*.

---

## Repair obligation, discharged here — the fourth table

**The source document's own opening sentence is false, and this is where it is repaired.** Rev 4's
preamble reads: *"All four disposition tables are at the end and are the changelog. Silence on a
finding is itself a defect, so every `F`, `X`, `B`, `M` and `S` has a row, including the ones this
revision refutes."*

Verified against the source this session: **only THREE tables exist** — rev 1 (35 findings),
rev 2 (`F1`-`F28`) and the scope extension (`X1`-`X25`). The file ends inside the `X` table at
line 1957. There is **no `B1`-`B6` / `M7`-`M13` table and no `S1`-`S12` table anywhere in it**. The
third-pass dispositions live only as inline `[B*-fix]` / `[M*-fix]` markers (concentrated in the
Metrics opener and the gate table, plus in-body parentheticals at D1, D4a, D6, D8, D11, D17, D19,
D20, D21, D23, D24 and §Sizing), and the seam dispositions only as inline `(S..)` annotations.

Two consequences, both applied:

1. **The `B`/`M` rows are harvested into a fourth table below**, each with the finding it answers,
   the disposition, and where the fix lives in the split corpus.
2. **The `S1`-`S12` rows are NOT recreated here.** They land once, merged with the logging plan's
   dispositions into one row per `S`, in `SEAM.md` (`seam/decisions-s1-s12`) — carrying them in
   two places is the exact defect this corpus was split to prevent.
3. **The preamble sentence itself is owned by `00-GOAL-TARGETS.md`**, which carries the H1 and the
   "four folded inputs" block. It must be re-cut there to read *"three disposition tables plus the
   third-pass table assembled at the split, and the `S` rows in `SEAM.md`"* — recorded here so the
   repair is not lost between two files.

---

## Answers to the review's open questions

1. **Which crate owns `Channel` / `GpuStage` / `PartitionGroup` / `ZoneId`, given `boyko_rhi_vulkan`
   may not depend upward?** `boyko_diag::profiling_abi` — a **new** zero-dependency bottom crate,
   not `boyko_utils` (which keeps its empty `[dependencies]`), because the logging plan needs the
   same leaf for the clock, the lane id and the loss vocabulary (S2). Two Cargo edges are added for
   this plan and both appear in the Integration table (F1/S2).
2. **Where does the fold actually run, and which of `Fixed×N` / `Main` is the primary CPU number?**
   At the top of `App::update_with_delta` (`app.rs:655`), the single funnel. The primary number is
   `__frame` — the whole `update_with_delta` body after the fold — with `__fixed_step` (N per frame,
   N recorded) and `__main_run` as children (F2/D16).
3. **What is the non-blocking source of `fence_seen`, expressed as an RHI verb?** There is none, and
   none is added. It is **derived** from `RenderEpoch >= slot.submit_epoch + FRAMES_IN_FLIGHT`, the
   asset-retire rule stated by `frame_driver.rs:255-262` and already an ECS `Resource` at
   `asset_refcount.rs:55` (F13/D4a).
4. **With `__gpu_null` measured at 0, what is G10's tolerance and what does "quantum" protect?**
   `__gpu_null` is deleted. The quantum is `measured_quantum_ns` — the GCD of the sitting's
   timestamp-derived values, means excluded — and it is a **sub-floor** of the band, never the band.
   G10's tolerance is the full band, and its licensing clause is `CommandWitness::stamp_positions` —
   a positional witness with no vocabulary, because the `ZoneId`-based one could not be compared
   against a collector that has none (F6/F17/D11a/M12).
5. **If `MAX_SYSTEMS` rises to 1024, is 128 KiB + 256 KiB acceptable, and what is `MAX_ZONES` sized
   against?** Yes, behind `feature = "profiling-analysis"` (dev-only, off in shipping). Zones are
   sized as `ENGINE_ZONE_SLOTS` (4096 dev / 256 shipping) + `user_zone_budget`, **all three
   profile-dependent** so the shipping `.bss` is 54 KiB rather than 284 (M10); per-system minting is
   tier-gated; exhaustion is **non-terminal** so a default-on feature can never panic a legal app
   (F5/C-III).
6. **What binds a `Floor` to its workload, and what forbids a caller-chosen sigma?** A `WorkloadTag`
   carried by both the `Floor` and every `LegSummary`, checked by `resolve`
   (`FloorWorkloadMismatch`); and `FLOOR_SIGMA = 3.0` is a `const` with no parameter anywhere in the
   API. `Floor::from_aa_control` is deleted; the in-sitting control becomes a separate `Twin` type
   with a fixed reduction (F4/D11).
7. **128-pair witness representation:** a `[u8; MAX_GPU_PAIRS]` mark array in `UnsafeCell` plus a
   single `AtomicU32 seal`. One plain byte store per bracket; one `Release` store **per frame**, not
   per pass (D5).
8. **Which thread runs retire:** the host thread, at the runner seam beside the `RenderEpoch`
   publication — **not** a `requires_dispatcher` system, which `system_meta.rs:130-141` shows would
   resolve to `SystemKind::CpuExclusive` and serialise the schedule every frame (F14/A3).
9. **Is `profiling` default:** yes, feature default-on so shipped code carries the sites;
   `ARM_MASK == 0` by default, so the runtime cost is one predicted-not-taken branch and the fold is
   `if mask == 0 { return }`. The tier is one column of the single `BOYKO_PROFILE` axis
   (`dev`=`Deep`, `shipping`=`Always`, `off`=feature off), five CI legs at rung 14, shared with the
   logging plan (S9). `SystemMeta.zone` is unconditional in both axes, so the 256 B pin is
   configuration-independent. **Amended at the split (S13):** "`ARM_MASK == 0` by default" is now
   load-bearing rather than incidental — it is the profiler's half of the RUNTIME axis, and the
   branch it leaves behind is the one cost a runtime flag **cannot** drive to zero. Only the
   compile-time ceiling deletes it. See `SEAM.md` §S13 and gate `GJ1`.
10. **Frame id of a sample:** not carried. Attribution is a **bidirectional** walk of `s.stamp` — a
    field every kind carries and no kind overloads (B1) — against `frame_begin_tsc[]`, stopping at
    the live frame's cut (A2). The region is *not* stamp-monotone (a nested span stamps at open and
    is written at close), which rev 3 asserted it was. Late arrivals older than the window are
    counted (`W9209`).

---

## Answers to the scope extension's questions

1. **Static vs game-defined instrumentation** — D19/D27. Two authoring paths, one registry, one
   store, one fold. A Rust plugin crate uses `declare_zone!` verbatim and pays the engine's 12 ns;
   only *data-defined* zones take the dynamic path, at ≤ 14 ns (≤ 18 ns across FFI). The engine path
   is protected by a **partitioned id space** (G11) and a **partitioned ring region** (G20) — and
   after B3 the partition is keyed on the **declaring crate** (one non-defaultable line at its
   root), not on which macro was used, so a plugin's *static* zones land in the user partition where
   the gates can see them.
2. **Volume** — D22/D23/D24. The ring stays fixed and lossy because that is what buys 12 ns;
   retention is three tiers; the stream is window-granular at 2.9 MB/h retail; drops are counted per
   class **and per region** in `u64` with a non-wrap proof, and they now force `NotResolved`.
   Decimation happens at retention and via the scope bit, **never at the call site**.
3. **Runtime configurability** — D20. 64 scope bits; the **only** public input is the kernel's enable
   bit on a **fieldless** `ProfilingScopeEnabled` tag (the macro rejects a fielded bitset tag — B2),
   with the `bit`/`name` payload in a separate `ProfilingScope` component, projected by **step 0 of
   the fold** — *not* by an observer, because `enable_tag_api.rs:77-88` documents that the enable
   path fires none. The write path is `commands.entity(e).enable::<ProfilingScopeEnabled>()` from any
   parallel system, applied inside the same schedule run. No public mask setter, no mirror, no dirty
   flag, no scheduled reconciliation system, no lock.
4. **Shipping builds** — D21/S9. Three tiers, one `const` ceiling, one **shared** `BOYKO_PROFILE`
   axis; `shipping` = `Always` + `profiling-analysis` off ⇒ **≤ 1 MiB profiler-attributable** (0.89
   computed, `.bss` included — M10), and jointly with the logger the figure `seam/joint-cost` owns —
   **not restated here**, because the restatement this row carried (≈ 1.99 MiB) rested on a logger
   half of 1.10 MiB that the logger's own files and `SEAM.md` both contradict. G14 is per-site +
   behavioural, not a self-contradicting symbol census (B5). Telemetry, crash diagnostics and a
   small counter set survive; per-system and per-pass zones, the contrast machinery, the concurrency
   analysis and the TOML writer do not. **Amended at the split (S13):** the ≤ 1 MiB figure and the
   joint one alike are **reserved extents committed at `arm`**, and `arm` is the enable path — a title that
   never turns the profiler on commits none of them. The word *resident* is correct only for the
   ARMED column. The re-cut rule and every sentence it touches are `seam/free-when-off`'s.
5. **Consumption by the game** — D25. `Res<Profiler>` from any system, a **published latency table**
   (CPU N−1, GPU N−4…N−2), `ProfiledZone` resolving ids once at setup, and a reference overlay at
   rung 15. Because the fold and the retire run outside the schedule, **no `SystemSet` and no
   ordering edge are needed**. The one refusal: the profiler is not an inter-system bus.
6. **Multi-process / replay / per-player** — D26. Session identity, build identity, an opaque player
   tag and replay correlation via `FixedTime::elapsed()` are **in**, at 44 B of header and 8 B per
   record. Cross-process aggregation and a live network viewer are **out**, with named re-entry
   conditions; remote arming is already served by (3).

---

## Open questions (remaining)

1. **`profiling-alloc` shim.** A global allocator is process-wide and perturbs everything it
   measures; the 19 zero-alloc gates answer "allocations per frame" more precisely, in test binaries,
   without perturbation. **Recommendation: build it, default off, `#[cfg]`-excluded at retail tier,
   artifact-labelled as a diagnostic mode whose numbers are not comparable to an unarmed run.**
2. **Artifact granularity.** The binary stream covers the "long capture" case, so the TOML can stay a
   document. Remains open only for a dev workflow that wants human-readable per-frame rows.
3. **v1.1 calibrated timestamps.** Deferred by D14. Two triggers now: a concrete cross-domain
   question ("is the CPU recording the frame, or waiting for the GPU to finish it?"), and an in-game
   overlay showing two axes, which will make users ask. Either must be answered with v1.1 or with a
   refusal, never with an uncalibrated offset.
4. **`Immediate` support on this box is unproven.** The design probes and records the resolved mode.
   If unsupported, rung 8's present-mode work reduces to labelling — **now stated in the rung table
   (D12), not only here.**
5. **Whether telemetry ever needs the log sink thread.** Decided against on the **corrected total**
   (350 µs = 2.1 % of one frame in 121, below this box's decidability floor — M7), not on rev 3's
   0.36 %, which omitted the reduction. Named trigger: `__telemetry_total` p95 > 500 µs on a real
   title, measured ⇒ hand off to `boyko_log`'s existing sink, **one thread for both subsystems, never
   two.**
6. **Scope namespace exhaustion.** 32 game-assignable bits. **Recommendation: refuse a second
   `ARM_MASK` word until a title actually exhausts 32**, because the second word costs the hot path
   and the first is not yet full.
7. **Histogram bucket geometry.** 3 mantissa bits / 6.25 % / 192 buckets / 400 B, chosen against the
   measured floor band (4.7-14.3 %). Widening to 4 bits is a config, not a redesign. Left open
   because no measured need exists.
8. **The in-tree lattice discrepancy.** `vg_occ_split_timing.rs:138` and `:881` say 16 ns; the
   odd-budget sitting measured 32 ns. This plan hard-codes neither and computes the quantum per
   sitting (S3). **Whether to repair those two prose sites is a separate, deliberately-scoped edit**
   — repairing doc-rot is as error-prone as writing it, and this plan is not the place.
9. **What the decoder becomes.** Rung 13 ships `prof_decode` as an in-tree CLI. Whether it eventually
   becomes a `boyko_ui` viewer inside the engine — the engine as its own Tracy, in-house, at zero
   dependency cost — needs no decision before rung 15.

**Moved out of this file, deliberately.** Rev 4's items 10 (*"the shipping diagnostics budget is
≈ 2 MiB, not 1"* — a **VALUES** call) and 11 (*`shipping-min` semantics* — a **SCOPE** call) are
**owner** calls, not architect calls, and they now live in `SEAM.md` (`seam/open-owner-calls`)
together with the logging plan's, plus a third that is new at the split: **how the enable flag
arrives**, given that `std::env::args`/`args_os` appear **zero** times in this workspace and every
runtime switch here is an environment variable. They are not duplicated here — burying an owner
call in a per-plan disposition table is how one gets missed.

---

## Checklist

Structure ✅ · Data structures ✅ (`repr`/align on every shared type; hot/cold split — `ZoneDesc` is
`&'static` and never on the emission path, `DynZoneHandle` carries only what emission needs; **every
size computed field-by-field and summed for four configurations, `.bss` statics included**;
false-sharing padding pinned by `const _` including the engine/user region split; **one record shape
whose attribution key is kind-independent**) · API ✅ (no `dyn`; no internal type in a signature;
kind-typed windows; **one `Floor` constructor, no sigma parameter, and a `const` reduction rule**; no
bare-delta constructor; no panicking accessor; no public mask setter; no `&str`-keyed emission; no
point-estimate quantile; **capability and data are separate components**) · Multithreading ✅
(per-datum table; **every ordering justified exactly once**; no `SeqCst`; partition = lane × region;
ten race-freedom clauses; `Send`/`Sync` incl. **`Profiler`'s explicit `unsafe impl`**,
`ZoneGuard: !Send` and `DynZoneHandle: Send + Sync`; no teardown, structurally; **no new thread, and
one fewer TLS slot jointly**) · Correctness ✅ (cursor wrap at both volumes with the corrected hours,
overflow-counter non-wrap proof, **`count: u32` non-wrap proof**, **no saturation path at all**,
dense minting under contention with an executable order, user minting with no leaked id, GPU deadline
**from a real non-blocking source, with a second horn for frozen submits and a guarded `grace`**,
teardown flush, frame attribution over three kinds and out-of-order stamps, sealing, forgotten guard,
panic unwind, multi-world, host-reset absence, clock epoch break, telemetry write failure **and
torn-block detection**, name-arena exhaustion, histogram saturation, empty-window `min`) ·
Integration ✅ (**two new Cargo edges plus the two shared ones, all named**; 27 files; 7 new module
groups; `Arena`/`ComponentPool`/`UnitId` untouched with a reason; 17 rungs each compiling alone, the
subtractive one isolated with **two** measured consumer lists and **two** mechanical gates that do
not rest on them) · Validation ✅ (**28 showable-RED gates, each with an explicit "cannot claim"**,
eight repaired from green-while-false in rev 3 and five more in rev 4; three gates deleted rather
than patched; tests assigned to rungs; 11 property tests; loom incl. the `arm` publication order;
Miri incl. `Profiler`'s `Send`/`Sync` and the accessor aliasing; 9 benches with regression thresholds
and a `config_tag` clause; a release-live list).

> **Carried verbatim, with one arithmetic discrepancy flagged rather than silently corrected.** The
> gate table in `profiling/05-LADDER-GATES.md` has **32 rows** (33 once `GJ1` lands), and the count of distinct
> base numbers is 26. Neither is 28. The "28" above is carried as written because it is part of the
> audit trail; the count note lives beside the table, where a reader can check it. Picking a number
> here would be a guess about which side rev 4 intended.

**N/A:** SIMD on the emission path — a 24 B record is two stores, and a 32 B AVX store would waste
8 B of ring per record; vectorisation belongs to A4 and is specified there. SIMD on `hist_fold` — a
scatter into 192 buckets with data-dependent indices; gather/scatter would be slower than the
8-instruction scalar form on AVX2.

---

## What rev 3 changed that rev 2 had EARNED — stated explicitly, each argued

Nothing below is a silent regression. Each is a rev-2 property that a finding or the scope extension
forced to move.

| Rev-2 property | Rev-3 state | Forced by | Argument |
|---|---|---|---|
| `WINDOW = 120` | **121** | S4 / P4-6 fact 7 | An even window makes every median the mean of two middle samples — a value no frame produced, half a lattice tick off. That is precisely how the 16 ns lattice was mis-derived |
| `MAX_SYSTEMS = 512` | **1024** | F5 | The kernel's own cap is `MAX_SYSTEMS_PER_SCHEDULE = 1024` (`schedule_builder.rs:70`). 512 silently truncated **both halves** of the headline concurrency statistic. Cost: `compat` 32 → 128 KiB, `intervals` 64 → 256 KiB, both behind `profiling-analysis` |
| Registry exhaustion **terminal** (`E9201`) | **non-terminal** (`W9201`, `DISABLED`, counted) | F5 / C-III | Per-system minting was unconditional on a default-on feature; a legal app with 1024 systems × 3 schedules would panic at build time. A missing *measurement* is not a wrong *answer*, so the `query_type_registry` precedent does not transfer |
| `resident ≤ 7 MiB`, proved by `debug_assert` | **four configurations**, ≈ 0.9 / 6.7 / 7.1 / 21 MiB, proved by a two-sided allocator + `VmReservation` gate | F26 / X5 | A `debug_assert` is gone in release and an artifact field is self-reported. G23 observes the allocator and asserts a **second** `arm` allocates 0 |
| `__gpu_null` as "the quantum probe" | **deleted** | F6 | Measured 0 on this box, every time. A measured-inert probe is not a probe, and keeping it made `resolve`'s `max(floor, quantum)` collapse to `floor` on the GPU channel |
| G10 tolerance "one quantum", simultaneous dual-record | **band-based, serial A/B, licensed by the record-order census** | F6 + F17 | The tolerance was 0 (unsatisfiable) and the configuration doubled the very command traffic D4 rejects the totality epilogue for. The census clause is the honest licence |
| Retire = a `requires_dispatcher` system | **a host-called function** at the runner seam | F14 | `requires_dispatcher` implies universal access ⇒ `SystemKind::CpuExclusive` ⇒ a full schedule serialisation point every frame, in the subsystem whose product is a concurrency statistic. Cost: less ECS-native in shape; precedent is the adjacent `RenderEpoch` line |
| Fold in `App::update` | **top of `App::update_with_delta`** | F2 | The windowed host calls `update_with_delta` directly (`runner.rs:1321`), so in the only configuration with a GPU channel the fold never ran |
| "the `Schedule::run` span" as the primary CPU number | **`__frame`, with `__fixed_step`×N and `__main_run` as children** | F2 | The frame is "Time → events → Fixed×N → Main" (`runner.rs:943`); that is not one interval |
| `Floor::from_aa_control(control, sigma)` | **deleted**; `Floor::from_session_file` only, plus a separate `Twin` | F4 | Two of rev 1's three refuted substitutions survived in it (one sitting, caller sigma), and no `Floor` was ever checked against the workload it licensed |
| `ARM_MASK` load `Relaxed` | **`Acquire`** | F11 | It gates the `buf` pointer. On x86-64 an `Acquire` load is the same `mov`, so the cost is zero instructions — rev 2's stated reason had the cost backwards |
| D2's three-branch per-emission lane resolution | **one TLS load**, set once per thread | F12 | The two specifications were incompatible and nothing set the TLS for workers, so every worker would have dropped |
| `instrument` as one number including an estimate | **`instrument_measured` + `instrument_estimated`**, only the former subtracted | F18 | Injecting a cross-binary median into a per-frame number, in the document that refuses to print unresolvable deltas |
| `CHANNEL_MASK: u32` | **`ARM_MASK: u64`** | X9/X10 | 64 scope bits; identical instruction count |
| `MAX_ZONES = 1024` fixed | **`zone_stride` fixed at arm**, tier-dependent engine slots + a dynamic budget | X2/X5 | A game must be able to declare its own zones; the store must be sized for them. Cost: one `imul` per sample at fold |
| Lane capacity 4096, one region | **2048 × two regions** | X4 | Region isolation is what makes G20's claim possible. Cost: engine burst headroom 4096 → 2048 samples = 5 frames, against a fold that runs every frame |
| `resolve` accepted any complete-looking window | **refuses drops and epoch breaks** | X8/C4 | This makes the engine side **stricter**: a bench that drops now produces no number instead of a wrong one |

**Rev 4's own further movement against rev 3**, recorded in the same spirit and argued in the B/M
table below: the region capacity moves 2048 → **1024** per region (B1's 24 B record would otherwise
put the dev slab at 7.5 MiB against a 7 MiB budget), so engine burst headroom is **2.5 frames**, not
5; and `count` widens `u16` → `u32`, taking the row from 19 to **21 B/zone/frame** and the columns to
**2.48 MiB** (M9).

---

## Findings disposition — rev 1 review (35 findings, carried forward)

| # | Finding | Disposition | Where |
|---|---|---|---|
| **B1** | `AtomicU128` does not exist | **FOLDED** — mark array `[u8; MAX_GPU_PAIRS]` + one `AtomicU32 seal`; one Release store per frame | D5, `FrameSlot` |
| **B2** | `WITH_AVAILABILITY_BIT` is `0x4`, not `0x20` | **FOLDED** — corrected against `ffi.rs:846,849`; G2c added | D4, G2c |
| **B3** | Floor: wrong instrument, wrong scope, wrong sigma | **FOLDED, then TIGHTENED in rev 3** — see F4 | D11, A6, G3a |
| **B4** | G5 vacuous — PINS are BMP SHA-256 | **FOLDED, then TIGHTENED in rev 3** — the armed clause is now an equality (F-gate-table) | D17, G5 |
| **B5** | No positive control for `Resolved` | **FOLDED** — G3b calibrated K vs 3K; also G2b | G3b, G2b |
| **B6** | `concurrency()` uncomputable from the store | **FOLDED, then CORRECTED in rev 3** — see F5, F19 | D9, G8 |
| **B7** | UAF on foreign lanes at disarm | **FOLDED** — no free ever; disarm is a mask store | D15 |
| **B8** | PR-5/PR-6 don't compile; test files unlisted | **FOLDED, then CORRECTED in rev 3** — see F1, F16 | Rung table |
| **M1** | Frame attribution / sealing undefined | **FOLDED** | A2 |
| **M2** | `depth` unincremented, UB in a `static` | **FOLDED** — debug-only TLS `OPEN_DEPTH` | D3a |
| **M3** | Two registries, no sync rule | **FOLDED, then MADE EXECUTABLE in rev 3** — see F9, F10 | D6 |
| **M4** | D2's premise false — the hazard is UNATTACHED→lane 0 | **FOLDED** | D2 |
| **M5** | Retire's thread specified twice | **FOLDED, then RE-DECIDED in rev 3** — see F14 | A3 |
| **M6** | Feature default unstated; fold uncosted | **FOLDED** | D1, target table |
| **M7** | The instrument sits inside its own primary number | **FOLDED, then RE-SITED in rev 3** — see F2 | D16 |
| **M8** | `WaveRecord.members` truncates above 256 systems | **FOLDED, then RE-BOUNDED in rev 3** — see F5 | D9 |
| **M9** | `rate_per_frame` panics | **FOLDED** — kind-specific windows | D13 |
| **M10** | `p10/p90` estimator undefined; control asymmetric | **FOLDED** | D11, `Contrast` |
| **M11** | No leg mechanism, no retention | **FOLDED** | A5 |
| **M12** | Layout fork asserted, not decided | **FOLDED, arithmetic CORRECTED in rev 3** — see F15 | D8 |
| **M13** | `calib_residual` = worst-of-N | **FOLDED** — `calib_cv` + `calib_rejected`; 20 ms window | D3 |
| **M14** | `hostQueryReset` fallback unspecified | **FOLDED** | D18 |
| **m1** | `total` sizing 4× off | **FOLDED** | `Profiler` |
| **m2** | `dur` saturation not recorded | **FOLDED** — `#[cold]` `Extension` sample | `Sample`, A1 |
| **m3** | Concurrent minting leaks ids | **FOLDED, then MADE EXECUTABLE in rev 3** — see F9 | D6 |
| **m4** | "no cold branch at all" is wrong | **FOLDED** | D6 |
| **m5** | 128 pairs exactly fills the witness | **FOLDED** — byte marks scale | D5 |
| **m6** | `min_max` vs `median`, one type | **FOLDED** | API |
| **m7** | p95 at n=120 is a 6th-order statistic | **FOLDED**; `n` is now 121 and odd (S4) | API, A4 |
| **m8** | `FrameRecord` undefined | **FOLDED, size CORRECTED in rev 3** — 88 B, computed field-by-field (F22) | Data structures |
| **m9** | `VK_NOT_READY` mapping unstated | **FOLDED** | D4 |
| **m10** | Artifact-at-exit loses partial evidence | **FOLDED** | API |
| **m11** | `Immediate` support assumed | **FOLDED**, and the rung's reduction is now stated in the rung table | D12 |
| **m12** | `SystemMeta` arithmetic 243 vs 244 | **FOLDED** — offset 242 / 244 total; `const _` assert | Invariant 2 |
| **m13** | Eight codes hard-wired to `eprintln!` | **FOLDED, then EXTENDED in rev 3** — `W9205` is once-per-window with a count (F20) | Integration |

**Note on rev-1's `m2`, carried so the trail is not broken.** Its fix — a `#[cold]` `Extension`
sample carrying the high bits of a saturating `dur` — **no longer exists in rev 4.** `Sample.value`
is a full `u64`, which deletes the saturation path entirely (B1). The row above is preserved as the
rev-1 record; the current answer is "there is nothing to record, because nothing saturates".

---

## Findings disposition — rev 2 review (F1-F28)

| # | Severity | Finding | Disposition | Where |
|---|---|---|---|---|
| **F1** | BLOCKER | RHI crates cannot see `boyko_ecs`; rungs 4-6 unbuildable; no Cargo change listed | **FOLDED** — the emission ABI moves to `boyko_diag::profiling_abi` (zero-dep leaf, `type_intern` precedent); **two Cargo edges added and listed in the Integration table**. The opaque-`u16` alternative is named and rejected because it reintroduces `VbTimedPass`'s hand-maintained table | §Crate graph, Integration |
| **F2** | BLOCKER | Fold in `App::update`, which the windowed host never calls; "the `Schedule::run` span" is not one interval | **FOLDED** — fold at the top of `update_with_delta` (`app.rs:655`), the single funnel; the primary number becomes `__frame` with `__fixed_step`×N and `__main_run` as children, and `fixed_steps` is recorded per frame | D16, A2, Integration |
| **F3** | BLOCKER | "Never blocks" has no showable red at the site that can violate it | **FOLDED** — `WAIT_BIT` is made **unrepresentable**: `const _: () = assert!(GPU_ZONE_QUERY_FLAGS & WAIT_BIT == 0)`, a compile error. The source gate is kept but re-scoped to the set of files naming `vkGetQueryPoolResults` | D4, G2a |
| **F4** | BLOCKER | `Floor::from_aa_control` re-admits two refuted substitutions; no floor↔workload binding | **FOLDED** — constructor deleted; `Floor::from_session_file` is the only one; `FLOOR_SIGMA` is a `const`; a separate `Twin` type carries the in-sitting control with a **fixed** reduction; `WorkloadTag` on `Floor`, `Twin` and every `LegSummary`, checked by `resolve` | D11, A5, A6, G3a |
| **F5** | BLOCKER | `MAX_SYSTEMS = 512` vs the kernel's 1024; terminal exhaustion can panic a shipping build | **FOLDED** — `MAX_SYSTEMS = MAX_SYSTEMS_PER_SCHEDULE = 1024`; `systems_unanalysed` counter for other schedules; exhaustion **non-terminal**; per-system minting tier-gated; id space split engine/dynamic | D6, D9, C-III, G11 |
| **F6** | BLOCKER | G10's tolerance is 0 on this box; `__gpu_null` is measured-inert | **FOLDED** — `__gpu_null` deleted; the quantum is `measured_quantum_ns` computed per sitting; G10's tolerance is the full band and its licence is the record-order census | D5, D11a, G10, S3, S6 |
| **F7** | BLOCKER | Principle 0: `static` + leaked std-heap `Box`, on a refuted precedent | **FOLDED** — the `EventBuffer` analogy is **withdrawn** (`events/event_buffer.rs:202` is a field, not a `static`); the argument is remade on its own terms (the emitters have no world); and the backing memory becomes **one `VmReservation`** owned by the `Profiler` `Resource`, with the ABI leaf allocating nothing | §Principle 0, D8, `Profiler` |
| **F8** | BLOCKER | "Zero cost when off" asserted, not proved | **FOLDED** — G1 becomes two-sided and token-level: feature-off `zone!(UNDECLARED)` must compile, feature-on the same source must not, plus an object-symbol census. The cost (a typo'd `Deep` zone name is invisible at retail) is stated. **Extended at the split:** G1 and G14 bound the COMPILE ceiling only; the RUNTIME flag's off-cost is `GJ1`'s, and the two are kept apart because only one of them reaches zero (S13) | G1, D1, GJ1 |
| **F9** | MAJOR | D6's minting sequence is not executable (`n` used before it exists) | **FOLDED** — five-step total order over real values, with the refusal path restoring the counter | D6 |
| **F10** | MAJOR | `ZoneHandle.id` ordering specified twice, incompatibly | **FOLDED** — one specification: `id` store `Release` / load `Relaxed`; the desc edge is `REGISTRY[i]` `Release`/`Acquire`, with the argument restated on the registry slot and why the `Relaxed` id load is safe | D6, ordering table |
| **F11** | MAJOR | Arm publication order unspecified; hot path stores through `buf` with no null check | **FOLDED** — order pinned (slab → `buf` `Release` → `ARM_MASK` `Release`); the mask load becomes **`Acquire`** (zero instructions on x86-64); `debug_assert!(!buf.is_null())`; a loom case | D1, A1, Multithreading |
| **F12** | MAJOR | Lane resolution specified twice; nothing populates the worker TLS | **FOLDED** — one specification: the lane TLS is written once per thread at its named sites; emission is one load + one compare. G7 gains a positive control that a worker's samples land in its own lane. **Corrected at the split:** there are **three** `set_lane` sites, not two — the third is `InstallGuard::drop` (`thread_pool.rs:279`), without which a panicking dispatcher stays `LANE_DISPATCHER` for the process (`substrate/lane-write-sites`) | D2, A1, G7, Integration |
| **F13** | MAJOR | `fence_seen` has no source; `frame_driver.rs:265` mischaracterised | **FOLDED** — the anchor is corrected (`submission_epoch` is a submit counter), and `fence_seen` is **derived** from `RenderEpoch >= submit_epoch + FRAMES_IN_FLIGHT`, quoting `frame_driver.rs:255-262`. No new verb | D4a, A3, §Constraints |
| **F14** | MAJOR | `requires_dispatcher` ⇒ `CpuExclusive` ⇒ a schedule serialisation point | **FOLDED** — retire is **not** a system; it runs at the host seam beside the `RenderEpoch` publication. The cost (a host-called function, less ECS-native in shape) is stated with its precedent | A3, D25, Integration |
| **F15** | MAJOR | D8's line arithmetic wrong and omits a column | **FOLDED** — recomputed: 19 B/zone/frame ⇒ 304 lines / 19 KiB at `Z = 1024` (rev 2 said ≤ 256 / 16 KiB and omitted `label`). The conclusion survives at ~6.6×; the "fits L1d" claim is qualified with the fold's own 6.4 KiB. **Recomputed AGAIN in rev 4 for `count: u32`** — 21 B/zone/frame, 336 lines, 21 KiB (M9) | D8 |
| **F16** | MAJOR | Rung-7 consumer list omits three production files | **FOLDED** — the list is re-measured (13 files, with `present/mod.rs:52-56`, `scene_types.rs:21/2631/2643/2655` and `swapchain.rs:14-16` named — **the middle field anchor is corrected from `:2645` to `:2643` at the split, re-verified against HEAD**), **and the rung is made not to rest on it**: the gate is a post-rung `rg` returning zero matches | Rung table, G24 |
| **F17** | MAJOR | Dual-record cross-check is confounded by its own instrumentation | **FOLDED** — dual-recording is replaced by a **serial** A/B (never both armed in one frame), and the licensing clause becomes the `CommandWitness` record-order equality rather than the timing | G10, rung 5 |
| **F18** | MAJOR | `instrument` mixes a per-frame measurement with a cross-binary median | **FOLDED** — split into `instrument_measured` (in-band) and `instrument_estimated` (with provenance); only the measured part is subtracted; `run_net` never contains the estimate | D16, `FrameRecord`, G9 |
| **F19** | MAJOR | G8's control unproducible; observed half self-overwrites; `sys` not derivable | **FOLDED** — (a) the pool/system configuration is pinned and a skip is a CI failure; (b) `intervals` becomes an **append** ring with `occ` and an `intervals_dropped` counter; (c) `ZoneDesc.system_index` + an arm-built `sys_of` side table | D9, A2, G8 |
| **F20** | MAJOR | `emit_diag` is `eprintln!`, per `LOST` pair, per frame | **FOLDED** — `LOST` is counted at the site and reported once per window with its count, the same rule as lane overflow. **Extended in rev 4:** the `eprintln!` seam is deleted outright; the profiler never prints from any path (S7), and the fold is the only `W92xx` emitter (S5/S6) | A3, D5, Integration |
| **F21** | MAJOR | `profile_spawn.rs` says ~20-30 ns **per pair**; the plan said 25 ns/call, 60 ns/pair | **FOLDED** — corrected to the measured text (`profile_spawn.rs:229-230`), and D1's rejection is restated on the corrected number, explicitly noting it is a 2× argument and no longer a 5× one | §Constraints, D1 |
| **F22** | MINOR | `FrameRecord` is 64 B, not 72 | **FOLDED** — the struct is redefined for D16's new fields and computed field-by-field at **88 B**, with a `const _` | `FrameRecord` |
| **F23** | MINOR | Cursor wrap is ≈ 49.7 hours, not 49 days | **FOLDED** — corrected, and stated for both volumes (49.7 h at 400/frame; 9.9 h at a game lane's 2000/frame), with a unit test driving the cursor across `u32::MAX` | Race clause 2 |
| **F24** | MINOR | Module path given three ways, all wrong | **FOLDED** — a path table in the header; the tree's actual `boyko_ecs::ecs::core::profiling` plus the new `boyko_diag::profiling_abi` (rev 3's leaf moved once more in rev 4 — S2) | Header |
| **F25** | MINOR | `ProfilerConfig.window` vs `const WINDOW` | **FOLDED** — `window` is removed from the config; `WINDOW` is a tier-independent `const`; re-arm with a different geometry ⇒ `E9213` | API |
| **F26** | MINOR | Resident-memory proof is a `debug_assert` | **FOLDED** — G23: a two-sided boot-total gate over the counting allocator **and** the reservation, plus "a second `arm` allocates 0". **Extended in rev 4 to a THIRD domain** (`.bss`, via `section_report`), because two domains could not see a static array at all (M10) | G23 |
| **F27** | MINOR | Rung 1 has no gate; tests are never assigned to rungs | **FOLDED** — every rung names its gates; unit tests are assigned to rungs | Rung table, Unit tests |
| **F28** | MINOR | When frames stop, neither retire horn fires | **FOLDED** — `flush_gpu` on the runner's teardown path (`runner.rs:261`) force-retires every slot as `Partial`, labels unavailable pairs `LOST`, counts `gpu_slots_abandoned` (`W9217`), release-live. **And the converse case — submits stop while frames continue — is M13's, folded separately** | D4a, A3 |

**Refuted / partially refuted findings — none silently.** Every `F` above is folded. Two carry a
correction *to the review itself*, stated here rather than left implicit:

- **F3's premise about scope is right; its implied fix ("cover every file that can reach
  `vkGetQueryPoolResults`") is insufficient alone**, because a grep over a growing file set is
  itself a maintained list. The const-assert is the primary mechanism; the file-set gate is the
  backstop.
- **F7's `EventBuffer` citation is right and this plan adopts it, but the review's path
  (`event_buffer.rs`) omits the `events/` directory** — the file is
  `crates/boyko_ecs/src/ecs/core/events/event_buffer.rs`. All `event_buffer.rs` anchors in this
  corpus carry the corrected path.

---

## Findings disposition — rev 3 review (B1-B6 BLOCKER, M7-M13 MAJOR)

**Assembled at the split from the inline `[B*-fix]` / `[M*-fix]` markers**, because rev 4 promised
this table and did not write it. Every row is sourced from the in-body text that carries the marker;
nothing here is inferred beyond it.

| # | Severity | Finding | Disposition | Where |
|---|---|---|---|---|
| **B1** | BLOCKER | **One 16 B record shape for three meanings.** `begin` meant *TSC at open* for a `Span`, *the value* for a `Counter`/`Gauge`, and *the high 32 bits of `dur`* for an `Extension` — and the fold reads that field **before** the kind dispatch, for the live-frame cut and the frame walk. A counter's payload was therefore consumed as a timestamp: a typical count (10³-10⁹) sits far below the cut (a TSC ~10¹³-10¹⁷) so **every counter sample landed in `drops.late`**, while a large one (a byte count, a handle) exceeded the cut and **truncated the whole region's fold for that frame** | **FOLDED, and the root cause named rather than the symptom.** The record becomes **24 B**: `stamp: u64` means "when" for **every** kind and is the only field attribution reads; `value: u64` gets its own 64 bits. **The same defect hit the `Extension` record, which the review did not name** — its dur-high-bits were also read as a TSC, so a span longer than `u32::MAX` ticks (*the hitch most worth recording*) silently lost its high word **and** was mis-attributed. A `u64` `value` **deletes the saturation path entirely**: no `Extension` sample, no `saturated` flag, no `#[cold]` second store, one compare-and-branch fewer in `Drop`, net instruction count **one lower**. **Costs stated, not hidden:** 2.67 records per 64 B line instead of 4 (0.375 line touches/sample vs 0.25) and, on a 64 B-aligned base, 2 of every 8 records straddle a line boundary; the alternative 32 B record is worse on both counts. The ≤ 12 ns budget is **re-gated against a baseline measured for this shape**, not re-asserted. Region capacity drops 2048 → **1024** per region (2048 would put the dev slab at 7.5 MiB against a 7 MiB budget), so engine burst headroom is 2.5 frames, and `G4` makes any shortfall visible | D1, `Sample`, A2, D19's cost paragraph; rung-1 and rung-2 unit tests; the three-kind frame-attribution property test |
| **B2** | BLOCKER | **A bitset enable tag may not carry fields, and rev 3's "only switch" had no caller.** Rev 3 used `ProfilingScope { bit, name }` as the enable tag; `reject_non_zst_bitset_tag` (`component.rs:580-604`) accepts only a fieldless struct. And `EcsMaster::enable`/`disable` take `&mut self` (`enable_tag_api.rs:87`, `:95`), which no parallel system can hold | **FOLDED — WITH A CORRECTION TO THE REVIEW THAT MAKES THE DEFECT WORSE, NOT BETTER.** Capability and data are split: fieldless `ProfilingScopeEnabled` (the kernel enable bit) + `ProfilingScope { bit, name }` (ordinary table storage on the same entity). The write path is `commands.entity(e).enable::<ProfilingScopeEnabled>()` from any **parallel** system, applied at that system's `apply` inside the same schedule run — no exclusive system, no serialisation point. **The correction:** the review placed the storage-kind `debug_assert_eq!` on `is_enabled`; in the tree it is on the **write** path (`set_enable_bit`, `:148-155`), and the **read** path `is_enabled → test_enable_bit` (`:201-215`) has **no assert at all** — it looks up `archetype.enable_store.column(tag)`, gets `None` for a non-bitset id and returns `false`. So rev 3 would not have panicked in debug; it would have projected an **all-zero `ARM_MASK` in every build, silently** — a profiler permanently disarmed with no diagnostic | D20, API, G12 clause 3, the `debug_assert!` list |
| **B3** | BLOCKER | **The partition was keyed on the wrong thing.** Rev 3 read "static ⇒ engine, dynamic ⇒ user" while recommending the *static* macro as the game path — so the recommended game path minted engine ids into the engine ring. A plugin with 3000 static zones exhausts the engine id range; a plugin looping a static zone overflows the engine ring. **G11 and G20 both passed anyway, because both exercised only `register_zone`** — the vacuous-gate shape: the gate's input class excludes the defect | **FOLDED.** The key becomes the **declaring crate**, stated once at its root and **not defaultable**: a crate that declares a zone without `profiling_partition!(Engine|User)` does not compile (unresolved `crate::__BOYKO_ZONE_PARTITION`), and `Engine` const-asserts `CARGO_PKG_NAME ∈ ENGINE_PACKAGES`. Two partitions, two counters over disjoint ranges, two SPSC ring regions whose region is a compile-time const (no runtime branch). **Both gates' REDs are now produced by the recommended game path** — a static `declare_zone!` in a `profiling_partition!(User)` crate — and each gate is two-crate by construction, because one crate can only be one partition. Residual, named: an out-of-workspace crate writing `profiling_partition!(Engine)` fails the const-assert; a workspace member that lies is one greppable line, pinned by a tidy test. **There is no per-site escape at all, which was rev 3's actual hole** | D6, D19, G11, G20; two rung-1 compile-fail tests |
| **B4** | BLOCKER | **Eleven `&'static mut` fields aliasing memory the same struct owns**, which Tree Borrows flags; and the `VmReservation` kept inside the `Profiler` `Resource`, whose `impl Drop` at `vm.rs:263` **unmaps** — so a world dropped in a multi-world test or at teardown dangles every published lane `buf`. That is the rev-1 UAF class re-entering through the *owner* instead of through `disarm`. Plus: no `unsafe impl Send + Sync for Profiler` existed at all, while `NonNull<u8>` is `!Send`/`!Sync` and `Resource: Send + Sync` (`resource.rs:42`, verified) | **FOLDED — with a LOCATION, because an argument cannot fix it.** The reservation is reserved, committed, published and then **deliberately `mem::forget`ed**, so it has **no owner and no `Drop` that could unmap it**; "never freed" becomes structural instead of asserted, and the address-space leak is the one deliberate leak, stated as such. The `Profiler` holds `base: NonNull<u8>` plus **byte offsets**, handing columns out through accessors that reconstitute a slice per call — the kernel's own precedent (`VmColumn` keeps `base: NonNull<T>` + accessors, `vm_column.rs:88`). No `Box<[T]>`, no `Vec`, no `&'static mut`. An explicit `unsafe impl Send + Sync for Profiler` carries three clauses (mutation only outside the schedule; in-frame access shared-only via `Res<Profiler>`; write-once base into a region never resized, moved or freed) and is **in the unsafe inventory and on the Miri list — rev 3 had it in neither** | D8, `Profiler`, D15, race clauses 8 and 9, the Miri-under-Tree-Borrows list |
| **B5** | BLOCKER | **G14's two clauses contradicted each other, so no RED was constructible.** Rev 3 asked one per-binary object-symbol census to report the recorder symbol **absent** (clause 1) and **present** (clause 2) at once. A census answers "is symbol S referenced in this object", per binary, and **cannot attribute a reference to a site** | **FOLDED by DELETION, not by patching** — rev 3's recorder-symbol census is in the "deleted rather than patched" list. G14 is replaced by **three clauses, none contradicting another**: (a) *per-site, token-level, across two profiles* — under `BOYKO_PROFILE=shipping` a `Deep` `zone!(NEVER_DECLARED_IDENT)` must **compile**, under `dev` the **same source** must **fail** (`trybuild`); (b) *behavioural liveness* — in the shipping binary, `__frame`'s span count > 0 over 10 frames, which is the clause rev 3 wanted, obtained from behaviour instead of from a symbol; (c) *a census only where a census can answer* — no reference to `ConcurrencyReport`, `resolve` or the TOML writer, all `#[cfg]`-removed with `profiling-analysis` | D21, G14, "Deleted rather than patched" |
| **B6** | BLOCKER | **Two gates asserted mechanisms that land three and seven rungs later.** G4 sat at rung 1 while claiming the `u64` accumulator + `fetch_sub` behaviour, which is rung 2's; G22 sat at rung 1 while naming `DYN_DESCS`/`DYN_NAMES`, symbols that do not exist until rung 10 — so it would have run against one symbol while its title claimed three | **FOLDED by SPLITTING, so each clause lands where it can fail.** G4 → **G4a** (rung 1: a full region refuses and counts — and it explicitly does **not** claim the accumulator), **G4b** (rung 2: the fold's accumulation is lossless, the same gate as logging's G11), **G4c** (rung 8: the loss reaches the reader). G22 → **G22a** (rung 1: `LANES` + `REGISTRY`), **G22b** (rung 10: `DYN_DESCS` + `DYN_NAMES`, plus the S12 compile-fail red). Each split gate states its own "cannot claim", and G4a's says in as many words that rev 3's single G4 *silently reduced to exactly this* at rung 1 | Gate table (G4a/b/c, G22a/b), rung table |
| **M7** | MAJOR | **The telemetry cost omitted its dominant term.** Rev 3 costed telemetry at "20-60 µs per 2 s" and benched `stream_encode` = *"400 `WindowRec`s + the `write_all`"* — but `WindowRec` carries `median` and `p95`, obtained by a strided gather over the frame-major columns **plus a sort of 121 values, per zone**. At a few hundred subscribed zones that is hundreds of gathers over a 2.48 MiB working set plus hundreds of sorts: plausibly 0.5-2 ms, synchronous, in-frame — an order of magnitude above the quoted number. **X25's "refused on the number" rested on a number that omitted it** | **FOLDED, and the refusal restated on the corrected total.** `count`/`total`/`min`/`max` stay O(1) folds for every subscribed zone; `median`/`p95` are carried **only** for `TelemetryConfig::quantiles`, capped at `MAX_TELEMETRY_QUANTILE_ZONES = 64`, beyond which a subscription is refused, counted (`telemetry_zones_refused`) and reported once (`W9218`); outside the subscription a `WindowRec` writes `NO_QUANTILE` in both fields — an explicit format value, **not a zero a reader could mistake for a measurement**. The reduction becomes its own zone (`__telemetry_reduce`), its own budget (p95 ≤ 150 µs at 64 zones) and its own bench leg; `telemetry_window` reports reduce, write **and their sum**, all three p95. Total budget ≤ 350 µs. X25 survives: 350 µs is **2.1 % of one frame in 121**, not 0.36 %, stated as a *spike* rather than amortised — and it is **below this box's own decidability floor** (4.7-14.3 %). Escalation trigger restated against the total: `__telemetry_total` p95 > 500 µs on a real title ⇒ hand off to `boyko_log`'s sink, one thread for both, never two | D23, A10, G26, the budget table, open question 5 |
| **M8** | MAJOR | **The telemetry stream had no framing at all.** `ZoneRow` is explicitly variable-length; nothing carried a length, a magic or a checksum; and `write_all` on `ENOSPC` returns **after a partial write**, so the file ends mid-record and a decoder cannot distinguish a torn tail from data. The round-trip property test would fail on any real disk-full file, and G15 explicitly disclaimed the one failure a player's full disk actually produces | **FOLDED.** Every record now sits inside a **self-delimiting BLOCK** — `{magic: u32, len: u32, seq: u32, crc32: u32}`, one per window, one `write_all`. Decoder behaviour is *specified*: a block with a wrong magic, a `len` past the bytes remaining, or a `crc32` mismatch **terminates the walk** and returns none of its records; the decoder reports `blocks_ok`, `records_ok` and `truncated_tail_bytes`. The round-trip property is restated as **byte-identical up to `truncated_tail_bytes`** — a property that holds on a torn file instead of failing on it. Framing costs 16 B per 2 s window; per-record framing was rejected at 8 B on a 40 B record (20 %) when there is exactly one `write_all` per window and therefore exactly **one** possible tear point. G15 gains clause (b), the injected short write, and clause (c), the restated round trip | D23, A10, G15, the property list, rung-13 unit tests |
| **M9** | MAJOR | **`count: u16` wraps silently within a single fold.** One fold consumes at most `LANE_COUNT × 2 regions × REGION_CAPACITY` = 80 × 2 × 1024 = **163 840** samples, and every one of them may target a single zone (a per-entity dynamic zone, a per-draw counter — precisely the "as much data as possible" case). Past 65 535, `total`/`min`/`max` describe a different sample set than `count` does, **no drop class covers it and no gate exercises it** | **FOLDED with a PROOF rather than a bound.** `count` widens to `u32`, which cannot wrap by the same arithmetic (163 840 ≪ 2³², and a cell is zeroed when its frame row is recycled) — so **`count_saturations` deliberately does not exist**. Cost: +2 B/zone/frame, +61 KiB retail, +230 KiB dev. The row total goes 19 → **21 B/zone/frame** (336 lines, 21 KiB at `Z = 1024`) and the columns to **2.48 MiB**, which is also where D8's L1d claim is re-qualified honestly at **30.6 KiB against a 32 KiB L1d** — tight, and measured by `fold_cost`'s `zone_stride` legs rather than assumed. Rev 3's "2.35 MiB" for the 19 B row is corrected too: that product is 2 353 664 B = **2.25 MiB**; 2.35 was the count in millions of bytes read as MiB | D8's layout table, `Profiler`, D22 tier A, D24b, the rung-2 100 000-sample test |
| **M10** | MAJOR | **The sizing rows omitted the `.bss` statics entirely, and named a method that does not exist.** Rev 3 carried the *dev* `.bss` figures into *both* configurations, leaving **234 KiB uncounted in the retail row** — which alone breaks the ≤ 1 MiB claim at 873 + 234 = **1107 KiB**. And it cited `VmReservation::reserved_bytes()`; the tree has `os_len()` at `vm.rs:190` and the type is `pub(crate)` (both verified), so no gate outside `boyko_ecs` could have called it either way | **FOLDED — and the fix is not to stop counting them.** The sizing table gains a `.bss` column with the breakdown written out: **shipping** = `LANES` 8 + `REGISTRY` 6 + `DYN_DESCS` 24 + `DYN_NAMES` 16 = **54 KiB**; **dev** = 20 + 56 + 144 + 64 = **284 KiB**. `MAX_USER_BUDGET`, `DYN_NAME_BYTES` and `ENGINE_ZONE_SLOTS` become **per-profile consts**, which is what makes the shipping row **908 KiB = 0.89 MiB** *with* the statics counted. `Profiler::reserved_bytes()` — a public accessor over `vm.os_len()` — replaces the non-existent kernel method rather than widening the kernel type's visibility. **G23 gains a THIRD domain** (`section_report` over the four statics), because its two existing domains, the std allocator and the reservation, **could not observe a static array at all** | §Sizing, D21's profile table, G23, `Profiler` API |
| **M11** | MAJOR | **`Floor.rel` was a scalar with no stated reduction.** Rev 3 said "all three repetition floors printed and never averaged" and then handed `resolve` a scalar without saying which of the three it was — the whole load-bearing question, given that the measured spread across four runs of this protocol is **6.3 / 14.3 / 4.7 / 13.5 %**, a 3× difference between candidate reductions. `min` or a mean rebuilds the false-win machine at a different scale while satisfying every arithmetic check | **FOLDED as a `const`-driven step, not a caller's choice.** `FLOOR_REDUCTION = Reduction::Max` is a `const` in the `floor` module; `from_session_file` applies it and **there is no parameter**. `max` is chosen because it is **the only reduction that cannot manufacture a win** — a floor is a claim about what this instrument *cannot* decide, and the honest scalar for that claim is the worst repetition, not the luckiest and not their average. "Never averaged" is preserved as a **different statement** from "never reduced": the session file and the `Floor` both carry all three (`rel_all`) plus which repetition supplied `rel` (`rel_source_repeat`), and the artifact prints all three. G3a gets a RED that moves **only** the reduction: a pinned three-floor fixture whose `min` is below and whose `max` is above an injected delta | D11, G3a, the rung-8 unit test |
| **M12** | MAJOR | **G10's licensing clause could not have been evaluated.** It compared `first_pair_of` between the two collectors — but `first_pair_of` is `[ZoneId; …]` and the old collector has no `ZoneId`, only `VbTimedPass` slots (`gpu_timing.rs:229`, `VB_PASS_COUNT = 10` at `:391`, both verified). The equality therefore needed exactly the hand-maintained `VbTimedPass → ZoneId` table D6 exists to reject — and a table written **alongside** the ported brackets makes the equality a tautology, "it agrees with itself" | **FOLDED with a witness that has NO VOCABULARY.** `stamp_positions` is the value of a monotone "commands recorded so far in this witnessed region" counter at the moment each timestamp is recorded. Both collectors produce it from the **same** instrumentation, so the licensing clause becomes `stamp_positions` (and its length) **identical between the two legs** — same timestamp count, each at the same position in the recorded stream. Shifting one bracket by a single command changes one entry. **No mapping table exists, so none can be wrong.** `first_pair_of` survives as the record-order witness *within* the new vocabulary. The witness's own perturbation is bounded: `stream_pos` must increment at *every* recorded `vkCmd*` in the region, so the whole `CommandWitness` sits behind `feature = "profiling-census"`, **default off**, enabled only in the G5/G10 gate binaries — the increments are host-side `u32` adds that record no command and change no device state, which is why a census build records the same command stream and why G5's byte-identity claim still speaks about the shipped configuration | D17, G10, the rung 4-6 unit test |
| **M13** | MAJOR | **The retire deadline could never fire in the host loop's actual failure mode.** F28 addressed "frames stop"; the loop behaves the other way round. `runner.rs:1328-1332` `continue`s on a 0×0 client **after** `update_with_delta` and **before** `wait_frame_in_flight`/record/submit (verified), so a minimised window keeps folding, keeps serving `Res<Profiler>` readers and keeps writing telemetry while `submission_epoch()` — hence `RenderEpoch` — is **frozen**. An epoch-only deadline can never fire there, and teardown is never reached because the process is alive. **And rev 3's A3 self-contradicted** on where retire is called ("between `wait_frame_in_flight()` and the record" vs "on the line that publishes `RenderEpoch`" — opposite sides of the `continue`). **And its grace decrement could underflow:** `… else if epoch_ok && slot.grace == 0 { retire } else { slot.grace -= 1 }` executes `0u8 - 1` when the epoch condition is false with `grace` already 0 — a debug panic, or in release a wrap to 255 that silently restarts the deadline for another 255 frames | **FOLDED — three changes.** (1) `retire_gpu` is called at `runner.rs:1320`, immediately after the `RenderEpoch` publication and **before** the 0×0 `continue`, so it runs on every iteration, minimised or not; the rev-3 contradiction is resolved in favour of the second site, and rev 3's `:1319` anchor is corrected to `:1320` (verified: `:1320` is the `RenderEpoch` assignment, `:1321` the `app.update_with_delta(dt)` call). (2) A **second, frame-counted horn**: `FrameSlot.record_frame: u64` and a `Partial` retire once `frame_now - slot.record_frame > GPU_FRAME_DEADLINE` (`= GPU_RING_DEPTH + RETIRE_GRACE_FRAMES + 2 = 8`) **regardless of the epoch**, counting `gpu_frame_deadline`. The two horns are independent: the epoch horn is tight in normal running, the frame horn fires when submits freeze. (3) The decrement moves **inside** the epoch arm and is guarded (`if slot.grace > 0 { slot.grace -= 1 } else { retire }`), with a `debug_assert!(slot.grace > 0)` before every decrement. G25 is the gate, with a RED for each of (2) and (3) | D4a, A3, G25, the rung 4-6 unit test, the `debug_assert!` list |

---

## Findings disposition — scope extension (X1-X25)

| # | Requirement / tension | Disposition | Where | Cost, stated |
|---|---|---|---|---|
| **X1** | Game-declared zones from a plugin crate | **RESOLVED WITHOUT A NEW PATH** — `declare_zone!` is exported from the leaf and re-exported through `boyko_ecs::prelude`; a Rust plugin pays the engine's 12 ns | D19 | none — **except** the one non-defaultable `profiling_partition!` line at the crate root (B3) |
| **X2** | Game-declared zones from data / config / script / mods | **FOLDED** — dynamic registry over static desc/name arenas; `DynZoneHandle`; `zone_dyn!`; `zone_dyn_open/close` for FFI | D19, D27, A7, rung 10 | ≤ 14 ns (≤ 18 ns FFI), 208 KiB BSS, budget declared at arm, **not tier-foldable** |
| **X3** | Game must not degrade the engine — **id space** | **FOLDED** — partitioned counters, disjoint ranges, independent exhaustion codes | D6, D19, G11 | one extra atomic counter |
| **X4** | Game must not degrade the engine — **ring capacity** | **FOLDED** — two SPSC regions per lane; region is a compile-time const; separate slabs and counters | D19, `ZoneLane`, G20 | lane control 8.5 → 17 KiB BSS; engine burst headroom 4096 → 2048 samples in rev 3, and → **1024** in rev 4 (2.5 frames) once B1's 24 B record landed |
| **X5** | Store width vs "as many zones as the game wants" | **FOLDED** — arm-time `zone_stride`; budget default 256, cap 3072; `W9211` above L1d; a wide-stride bench leg | D8, G-bench | one `imul` per sample at fold; dev resident 6.7 → 21 MiB at the cap |
| **X6** | Hours of data vs a fixed lossy ring (**C-I**) | **FOLDED** — three retention tiers: the 121-frame ring + lifetime accumulators + opt-in log-linear histograms | D22, rung 12 | +24 KiB always, +25 KiB at 64 hist slots; **no per-frame history beyond ~2 s, ever** |
| **X7** | Drop count must stay honest at session scale | **FOLDED** — `fetch_sub(observed)` clear, `u64` accumulators, per-class **and per-region** attribution, a non-wrap proof, release-live counters (18 classes in rev 4) | D24, G4 | none. **Open, and it travels with `substrate/loss-fold`:** `fetch_sub` closes the CONSUMER side; the PRODUCER-side lost-update window is BLOCKER Q2 and is unanswered |
| **X8** | A silently truncated capture is the vacuous-gate pattern | **FOLDED, AND IT TIGHTENS THE ENGINE SIDE** — any leg with a drop or an epoch break is `NotResolved { reason }` | D11, D24d, A5, G13 | **a bench that drops now produces no number instead of a wrong one** |
| **X9** | Runtime toggling without restart or a hot-path lock | **FOLDED, BUT THE PROPOSED MECHANISM IS REFUTED** — `ProfilingScope` + kernel `IsEnabled` stays; the **observer does not** (`enable_tag_api.rs:77-88`: "no hook / observer fire"). Projection is step 0 of the fold | D20, A8, G12, rung 11 | ≤ 5 ns × `scope_count` per frame (≤ 320 ns at 64 scopes), inside `instrument_measured`; toggle latency one frame |
| **X10** | Per-subsystem granularity | **FOLDED** — 64 scope bits: 0..7 channels, 8..31 engine, 32..63 game | D20 | 32 game bits; a hierarchy is refused to keep the gate one `bt` |
| **X11** | Shipping builds — what survives | **FOLDED** — `ZoneTier {Always,Dev,Deep}` + `const GLOBAL_TIER`; `profiling-analysis` split; retail ≤ 1 MiB; a per-site + behavioural gate. **Rev 4 replaces the `option_env!`/per-crate `build.rs` route with the single shared `BOYKO_PROFILE` axis read by `crates/boyko_diag/build.rs`** (S9), and `crates/boyko_ecs/build.rs` is NOT created | D21, G14, rung 14 | profile change rebuilds the workspace from `boyko_diag` up; ~12 KiB dead `.bss`; a `Deep` zone-name typo is invisible at retail; CI 1 → 5 legs (4 net new, **shared** with logging) |
| **X12** | Player telemetry / crash diagnostics / support log | **FOLDED** — append-only binary stream, window-granular, synchronous, 2.9 MB/h retail, ≤ 2 s loss on hard kill, rotation, loud failure handling. **Framed in rev 4** (M8) | D23, A10, G15, rung 13 | one `write_all` per 2 s; **the corrected budget is reduce + encode + write p95 ≤ 350 µs**, not the encode alone; inside `instrument_measured` |
| **X13** | Consumption from ECS systems while the frame runs (**C-IV**) | **FOLDED, AND SIMPLIFIED** — `Res<Profiler>` + `ProfiledZone` + a **published** latency table (an artifact field, not a printed line — S1). Because the fold and retire run **outside** the schedule, the extension's `ProfilerSet::{Retire,Read}` pair and its ordering edge are **not needed and are dropped** | D25, rungs 11/15 | CPU data is N−1, GPU N−4…N−2, by construction |
| **X14** | "Gameplay code reads its own counters to make decisions" | **SPLIT: half folded, half REFUSED** — windowed statistics driving LOD / dynamic resolution: supported. Same-frame counter readback as a message bus: refused (a shared-line RMW on the hot path or a mid-frame fold; the ECS already has events and resources) | D25, D28 | the game samples its own ECS-owned datum once per frame via `gauge!` |
| **X15** | Debug HUD via `boyko_ui` | **FOLDED** — reference overlay at rung 15, zero-alloc gated with a positive control, `ZoneId` resolved once at setup | D25, G19, rung 15 | none |
| **X16** | Sampling / decimation policy | **DECIDED, ONE FORM REFUSED** — decimation at retention and via the scope bit; **no 1-in-N gate at the call site** (a per-site RMW on a shared line) | D22, D28 | a game wanting 1-in-N implements it in its own code, visibly |
| **X17** | Per-session / per-player identity | **IN** — `session_id`, `run_id`, `build_hash`, opaque `player_tag[16]` the engine never interprets. **Rev 4: the id is `boyko_diag::SessionId`, minted ONCE and shared with the logger's artifact header** (S11), so the two files join | D23, D26 | 44 B of header |
| **X18** | Save / replay correlation | **IN, at 8 B/record** — `fixed_elapsed_ns` = `FixedTime::elapsed()` (`fixed_time.rs:162`, verified), the kernel's determinism witness | D26, A10 | none |
| **X19** | Multi-process / networked aggregation | **OUT, argued** — needs cross-machine clock correlation D14 refuses to fake on one machine, plus a transport the engine lacks; the merge is a tool over files that already share `SessionId` + `fixed_elapsed_ns` | D26 | re-entry condition named |
| **X20** | Live network viewer / remote streaming | **OUT, argued** — the Tracy protocol renamed (D10), plus a socket in the frame loop | D26, D28 | a tailed file answers the same question at zero engine cost |
| **X21** | Remote arm/disarm switch | **ALREADY SERVED** — a network handler toggles the scope like any other system. **Corrected in rev 4 (B2):** the call is `commands.entity(e).enable::<ProfilingScopeEnabled>()` from a parallel system (or `world.enable::<ProfilingScopeEnabled>(e)` where `&mut EcsMaster` is held); rev 3's `world.enable::<ProfilingScope>(e)` names a fielded tag the macro rejects and a receiver no parallel system can hold | D20, D26 | nothing to build |
| **X22** | Session-scale clock hazard (suspend/resume) | **FOLDED — a hazard a 121-frame horizon cannot meet by itself** — forward-jump detector, window discard, epoch counter, `#[cold]` recalibration, `W9216`, `NotResolved { EpochBreak }`. **Rev 4: the clock and its epoch are `boyko_diag`'s, shared with the logger** (S4), and G21 asserts on **both** artifacts | D3, A2, G21 | one 20 ms hitch after a resume |
| **X23** | Lane capacity in a retail process | **FOLDED** — a tier-dependent `const`, keeping the ring mask an immediate (a runtime capacity would add a hot-path load). **Rev 4: it comes from `boyko_diag/build.rs`'s single `BOYKO_PROFILE` axis, so `boyko_ecs` does NOT gain a `build.rs`** (S9); rev 3's cost line is superseded | D21, D8 | the const lands in the shared leaf, not per crate |
| **X24** | Two panic hooks (profiler + logger) | **REFUSED — one process-global hook** — the logging plan owns it (`PRE_FLUSH: [AtomicPtr<()>; 8]`, claimed by CAS); the profiler **registers** `#[cold] flush_on_panic()` there and installs nothing. `flush_on_panic` takes no arguments and must not touch the `World`, which is why the telemetry double buffer and its file handle are a `boyko_app` process-static rather than `Profiler` fields | Lifecycle order (`SEAM.md`), D23 | without the logging crate, the ≤ 2 s telemetry loss bound stands unaided; the registrant bound is asserted **per registrant**, not proved in general |
| **X25** | A second sink thread for telemetry | **REFUSED ON THE NUMBER — and the number was WRONG.** Rev 3 said 20-60 µs per 2 s = 0.36 % of one frame in 120; M7 shows that omitted the window reduction, the dominant term. The refusal is restated on the corrected total: **350 µs = 2.1 % of one frame in 121**, a *spike*, and one **below this box's own decidability floor** (4.7-14.3 %). The engine's only threads stay the pool's | D23, open q5, G26 | escalation trigger restated against the **total**: `__telemetry_total` p95 > 500 µs on a real title ⇒ hand off to `boyko_log`'s sink, **one thread for both, never two** |

**Two parts of the extension are refused as bad ideas for this engine, not deferred:**

1. **The `IsEnabled` observer projection (X9's mechanism).** It cannot be built: the kernel
   deliberately fires nothing on an enable-bit toggle, and that absence is what buys the O(1) warm
   toggle (`enable_tag_api.rs:77-88`). Adding a fire would tax every `EnableTag` user in the engine
   to serve one subsystem. The fold-step projection costs ≤ 320 ns per frame at the 64-scope
   maximum, is disclosed inside `instrument_measured`, and needs no kernel change.
2. **Same-frame counter readback as an inter-system bus (X14).** It is a shared-line RMW on the
   emission path or a mid-frame fold, either of which destroys the 12 ns budget the whole design is
   built around — to duplicate a capability the ECS already has in events and resources.

---

## Seam dispositions (S1-S12) — NOT here, and that is deliberate

`S1`-`S12` are the seam decision record's, and both plans owed a disposition for each. Carrying two
copies is precisely the failure this corpus was split to prevent — a reader cannot tell which is
current and finds out only by acting on the stale one. **They live once, with both plans'
dispositions merged into one row per `S`, in `SEAM.md`** (`seam/decisions-s1-s12`). `S13`
(free-when-off) is not in that record at all: it is an **owner requirement folded in at the split**
and is numbered there so it is discoverable beside the others.
