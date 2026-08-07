# Logging — dispositions, changelogs, edge cases and open questions

<!-- CONTRACT
exports:  logging/dispositions
assumes:  seam/decisions-s1-s12
assumes:  seam/open-owner-calls
assumes:  logging/goal-and-audiences
assumes:  logging/ladder
assumes:  logging/gates
-->

> Carved from `docs/LOGGING-SYSTEM-PLAN.md` (v4) — the two changelogs, the answers to the first review's eight questions, §Edge cases, §Open questions, §Checklist, all three findings-disposition tables with their "Refuted, with the evidence" sections, and the scope-extension disposition. Diff against that document until the monolith is retired.
>
> **NOT carried here: §Seam disposition (S1-S12).** It moves to `../SEAM.md`, where it is merged with the profiling plan's inline S dispositions into one table with one row per S. Carrying it in two places is the exact defect this corpus was split to prevent. Its closing paragraph — the two items that are "open, and NOT this document's to close" — moves with it and is reached through `seam/open-owner-calls`.

This is the audit trail. Silence on a finding is itself a defect, so every B, F, M, N and X has a
row, including the ones the design refutes.

---

## Changelog v3 → v4

v4 folds **two** inputs. They stay separable: §Findings disposition (v3 → v4) is the defect half,
the seam disposition (now `../SEAM.md`) is the cross-plan half. Where they collide the collision is
named at the point of collision, not in a table.

**Input 1 — the third-pass review (REJECTED, 10 blockers + 4 majors).** Five of them are outright
design holes, fixed by structure and not by prose:

- **B1** — `LogRing`/`LogCensus` hold a `VmColumn`, and
  `crates/boyko_ecs/src/ecs/memory/vm_column.rs:70` states verbatim that `VmColumn` is **NOT
  `Send`/`Sync`**, while `crates/boyko_ecs/src/ecs/core/resources/resource.rs:42` requires
  `Resource: 'static + Send + Sync + Sized`. v3's fold therefore did not compile — F7's failure mode
  one level up. v4 writes the SEND10-shaped `unsafe impl` with the exclusivity argument naming every
  holder of `&`/`&mut`, and **removes the sink thread from that set entirely**
  (`logging/game-facing-surface`, B1 block), pinned by a `const _` `assert_send_sync` check.
- **B2** — the sink→ECS "handoff ring" was referenced three times and defined nowhere. v4 specifies
  **`ECS_HANDOFF`** as a first-class SPSC byte ring reusing `LogLane`'s own wrap rule, with layout,
  capacity, ordering, overflow accounting (`W0117`), budget row and SAFETY clause
  (`02-SINK-LIFECYCLE.md`). It is what makes B1's argument true: the sink writes `ECS_HANDOFF`,
  never `LogRing`.
- **B5** — the crash drain CAS'd `SINK_STATE` out of `{Exited, NotBooted, Manual}` and called all
  three quiescent. `Manual` is not: it means an arbitrary thread may be inside `drain()` **right
  now**. v4 CASes the **consumer role itself** — `DRAIN_OWNER`, claimed by the sink thread, by
  `drain()` and by the crash drainer alike (Decision 24) — and G14 gains the leg that panics with a
  manual drain in flight.
- **B8** — `shipping-min` was structurally guaranteed to contain the session's *beginning*: `Manual`
  never drains, admission control drops new records, so 32 × 16 KiB fills with boot records within
  seconds and everything up to the crash is refused. v4 gives the profile a real consumer:
  `log_drain_system` takes `DRAIN_OWNER` and runs the drain itself once per frame, on the frame
  thread, with the cost stated (Decision 10, Decision 25). The retained window is restated **in
  records**.
- **B10** — `LogPod`'s blanket `copy_nonoverlapping` of `size_of::<Self>()` copies padding bytes,
  and the sink then materialised a `&[u8]` over uninitialised memory — UB independent of whether
  `POD_LEN` was honest. v4 deletes the blanket copy: the trait requires `encode_pod`, the derive
  generates it **field-by-field** through `LogValue`, and `POD_LEN` is a const sum of field lengths
  rather than `size_of` (Decision 19b).

Three more were vacuous or misplaced gate legs — this campaign's recurring defect, now caught
inside the very gates written to catch it. **B3**: G17's leg (c) could not go red, because the F6
overrun is intra-lane by construction and no neighbouring canary is reachable; it is replaced by an
*undrained-record* assertion at a **second, explicitly named fill level**, because the two legs need
different fills and v3 named one. **B4**: G4's sampling leg asserted 500 for a quantity that is 1000
(arguments are evaluated before the sample decision) and needed a mechanism that lands 12 rungs
later; the observable is split and the leg moves to L12-gate as **G10e**, where the two numbers
together are the claim. **B9**: three mechanisms promised durability and wrote stderr;
`write_oracle_line` now fans out to **every configured synchronous destination**, including the
crash file, and the "durable-on-write" claim is restated against what a `write_all` actually
guarantees.

**B6** and **B7** are corpus-and-walker defects measured directly against the tree; both fixes are
stated in measured numbers rather than in shapes (Decision 6). **M1-M4** are folded: the per-code
`fired=` figure becomes a real per-**site** observation through an intrusive `ONCE_SITES` list (and
`RateSlot::fired` is deleted as dead); the integer audit gains `seq_lo`'s reconstruction rule and
every `BinarySink` width; code-index exhaustion gets a defined, never-aliasing return; and the
census's `vk-validation` showcase row — inert by construction, since Decision 9b guarantees no
record ever reaches that target — is **dropped**.

**Input 2 — the seam decision record (`boyko_diag`).** A new zero-dep bottom crate owns the clock,
lane identity, the loss vocabulary and the never-freed-storage policy; both this plan and the
profiling plan consume it. The three edits that cost this plan the most:

- **S1** — **`report!` is deleted from this plan**, with test 16 and L8b's 20 measurement rows. The
  profiler owns the measurement channel end to end; its durable output is an artifact, never
  stdout. The migration denominator falls from ≤ 98 to **≤ 78**. `OUT_LOCK` **survives** — it still
  serves `write_oracle_line`, the sync-routed targets and `SINK_REQ` — so Decision 9c stays whole
  and G18 keeps its subject.
- **S3/S4** — lane identity and the clock both move to `boyko_diag`. This plan's `MAX_LANES`, its
  `hash(thread_id)` claim scan, its `Drop`-guard TLS, its retire protocol and `tsc.rs` are deleted;
  `W0101` is struck. The honest consequence, recorded where the number lives and not only here:
  deleting the `Drop` guard takes "**≤ 1 allocation on a thread's first emit**" to **0**, so the row
  that existed to be honest about a cost now records that the cost is gone.
- **S9** — one compile axis, `BOYKO_PROFILE`, owned by `boyko_diag/build.rs`.
  **`crates/boyko_log/build.rs` is not created.** Decision 25's table loses its `GLOBAL_CEILING`
  column (a *runtime* preset cannot deliver a *compile-time* const) and the preset is renamed
  **`LogRuntimePreset`**; the header prints `build_profile` / `runtime_preset` / `ceiling` as three
  independent facts.

**The joint cost is absorbed, not deferred.** "Producer working set ≤ 4 cache lines" is true **in
isolation only**; with the profiler armed the joint figure is **7-8**, and that sentence now sits
next to the number in §Performance targets, not only in a disposition table.

**What did NOT change:** deferred formatting, the POD record + `&'static LogSite`, `.bss` statics
with `Off == 0`, the SPSC lane ring itself, B1's staged-copy drain, B3's shared wrap rule, B8's
withdrawal of the validation migration, M23's tidy-test-primary enforcement, Decision 9c's
`OUT_LOCK` protocol, and every v3 fold the third-pass review did not name.

**Added after v4, at the corpus split:** **S13**, the owner's free-when-off requirement. It is not
part of the round-3 seam review and a reader of that review will not find it there. It is specified
once, in `../SEAM.md`, and reaches this plan as four re-cuts: `boot()` spawns no thread and installs
no hook (they move to `enable()`); the crash file is opened on the enable path rather than "at
boot"; every `.bss` figure in this plan is labelled a **reserved extent** rather than a resident
cost; and gate **GJ1** measures the off-cost with a control leg that can invalidate its own
instrument.

---

## Changelog v2 → v3

v3 folds **two** inputs at once. They are kept separable on purpose: a reader who only cares about
the defects can read §Findings disposition (v2 → v3), and a reader who only cares about the new
audience can read §Scope-extension disposition. Where the two collide, the collision is named in
the audience-conflict table (`logging/goal-and-audiences`) and decided there.

**Input 1 — the second-pass review (REJECTED, 26 findings).** Two of them are outright correctness
bugs and are fixed by arithmetic and by a type size, not by prose:

- **F6** — `free = limit - (w - read_cached)` **underflows in `u32`** exactly in the state the Error
  reserve is designed to produce, and the producer then overruns live ring bytes. Fixed by
  reformulating admission control as `avail = CAPACITY - used` (an induction that cannot go
  negative) and applying the reserve with `saturating_sub` (Decision 5, Algorithms A6). Gate **G17**
  exists solely to red on the old arithmetic.
- **F7** — `LogRing`'s `VmColumn<LogLine>` **panics at construction**:
  `crates/boyko_ecs/src/ecs/memory/vm_column.rs:144-149` asserts `COMMIT_GRANULE % size_of::<T>() ==
  0`, `COMMIT_GRANULE = 64 KiB` (`crates/boyko_ecs/src/ecs/constants.rs:7`), and v2's `LogLine` was
  12 bytes. `LogLine` is now **16 bytes, `Copy`, with a `const _: () = assert!` beside the
  definition** so a future field addition fails the build instead of the plugin
  (`logging/game-facing-surface`).

Four more were "a gate that cannot fail" in a new costume — the campaign's own recurring defect —
and each is either given a showable red state or **deleted**: test 17 (F1), G7's negative leg (F2),
P1 (F3), G2's thread/hook legs (F4), registry checks 3/6 (F5). `OUT_LOCK` grows a **bounded,
re-entrancy-aware, unwind-safe** protocol (F8) and is **not** registered in
`docs/HOT-PATH-EXCEPTIONS.md`, because registering it **reds CI** (F9 —
`scripts/check_hotpath_exceptions.py:337-341` matches registry rows against
`#[allow(clippy::disallowed_types)]` counts per file, and an atomic carries none).

**Two findings are REFUTED with evidence, and one is refuted in part** — see below. v2 said
"Refuted: none"; that was a disposition, not a virtue, and it is not repeated for its own sake.

**Input 2 — the scope extension (games, not just the engine).** Dynamic targets minted from data
(Decision 18), downstream code tables (Decision 19), game-defined POD values (Decision 19b),
per-target sampling (Decision 20), a session-scale integer audit (Decision 21), a binary sink that
does not format (Decision 22), runtime sink/level control with no restart and no lock (Decision 23),
a crash drain (Decision 24), four shipping profiles (Decision 25), and the in-frame reader surface a
`boyko_ui` HUD and a telemetry reducer consume (Decision 17, Decision 26). Seven asks are **refused
with reasons** rather than designed (§Refused).

**What did NOT change, because the review said to keep it:** deferred formatting, the POD record +
`&'static LogSite`, `.bss` statics with `Off == 0`, the SPSC lane, `report!` as a separate
synchronous channel, B1's staged-copy drain, B3's shared wrap rule, B8's withdrawal of the
validation migration, N30's honesty about `W0101`, and M23's tidy-test-primary enforcement.

### Answers to the first review's eight questions (carried forward; two amended by v4, marked inline)

1. **Drain order** — the drain copies each record out of the ring into a staging arena, *then*
   advances `read`, *then* sorts, *then* decodes from staging. `read` never advances over bytes the
   sink still intends to read. (Algorithms C)
2. **Encoded length** — `LogArgs::encoded_len(&self) -> usize`, a runtime method that const-folds
   for all-fixed tuples. `&str` encodes as `u16` length + bytes. `fmtv` no longer exists; a
   `Display` is rendered by `dsp!` into a caller-owned stack buffer **in argument position**, so the
   ring is never open while user code runs, and an overrun is a truncation of a `&str` that has
   already been produced. (Decision 1a, Decision 13)
3. **Wrap** — records never straddle. Deterministic shared rule: `LANE_BYTES - off < HEADER_BYTES ⇒
   both sides wrap`; otherwise a PAD record (null `site`, `len` = tail) consumes the tail.
   (Algorithms A6)
4. **Re-entrancy** — forbidden *structurally*: nothing between lane acquisition and the `Release`
   store can call user code, because argument encoding is over already-materialised POD and `&str`,
   and `LogPod::fmt_pod` runs on the sink. A re-entrancy `debug_assert` guard backs it; test 24
   asserts it. (Decision 13, Decision 19b)
5. **Sink rate** — stated as a design number (≥ 500 K records·s⁻¹ **aggregate**, text sink, at the
   default geometry) and **gated at L3** by `sink_sustained_rate`, which must show the drop knee.
   (Decision 10, `logging/ladder`)
6. **Perturbation** — ABBA-counterbalanced logger-on/logger-off with an interleaved zero control in
   the same sitting. **v3 changed the instrument** (headless schedule bench, not windowed frame
   time, which FIFO clamps); **v4 changes the leg matrix**: P1 is a 2×2 of {logger off, on} ×
   {profiler absent, armed}, because a perturbation measured with the other subsystem in an unstated
   state is not a measurement of either (gate P1, F3, S10).
7. **`[vk-validation]`** — stays synchronous. The migration is withdrawn, **and so is v2's remaining
   one-line edit** (Decision 9b, F12). **v4 additionally deletes the census's `vk-validation` row**,
   which was inert by construction (M4).
8. **Handle / flush-without-consumer** — `LogHandle` is deleted; `boot()`/`shutdown()` are free
   functions over process-lifetime statics. `flush()` reads `SINK_STATE` first and returns
   `FlushResult::NoConsumer` immediately when nothing can ever acknowledge. There is no
   `join`-with-timeout because std does not provide one; shutdown observes a sink-exited atomic with
   a bounded spin and then **detaches**. (Decision 12)

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
| E16 | Drop order at shutdown — **this row DECOMPOSES `shutdown()`; it does not compete with the caller-level order.** Everything after `shutdown()` below is an INTERNAL of that call, not a fourth thing the host invokes. The host-level sequence is owned by `seam/lifecycle-order` and reads `flush_gpu()` → `disarm()` → `flush()` → `shutdown()`; the two COMPOSE. *(Two independent readers have now read this row as a transposition of the host order. It is not one — but a row that misleads two readers is a defect in the row, so the scope is stated here rather than left to be inferred.)* | `flush_gpu()` → `Profiler::disarm()` → `shutdown()` → `PRE_FLUSH` callbacks → `flush()` → `Exiting` → unpark → bounded spin on `SINK_EXITED` → detach on timeout → sinks close. **`flush_gpu` ahead of `flush` is the whole fix** for GPU-side diagnostics emitted after the logger stopped accepting them (S5). Idempotent; safe from `App` teardown, the panic hook and process exit. |
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
| **E22** | **Hard crash — `abort()`, `SIGSEGV`, guard-page stack overflow** | The panic hook does not run; whatever is in the lanes and in `SINK_OUT` is lost. **No design in this crate survives it**; making every record durable is a syscall per record. Bounded, not fixed, by three mitigations **whose limits are now stated rather than implied** (B9): (i) the per-target sync bit — `write_oracle_line` fans out to the crash file opened on the enable path, so in `shipping`/`shipping-min` the bytes actually leave the process instead of going to an unconfigured stderr; **"durable" here means one `write_all`, not `fsync`**, which is opt-in via `sync_durable` at ~0.1-10 ms; and under `OUT_LOCK` contention the cost is bounded only by the 50 ms steal deadline, at which point the line may interleave. (ii) `flush_interval_ms` (default 1000 in `shipping`). (iii) The crash file, which at least exists and carries the session header **and, under `Scheduled`, holds records adjacent to the crash rather than to boot** (B8). Stated, not worked around. |
| **E23** | **Sampled capture aliases a periodic emitter** | `1/2^k` is strided, not random. Per-lane seeding breaks aliasing across lanes, not within one. The census prints `sampling=1/N (strided, not random)` and `boyko-W0113` fires once per sampled target, so the bias is in the log rather than only in a footnote. |
| **E24** | **A `CallbackSink` blocks** | The sink thread stalls; lanes fill; drops are counted and reported. This is the stated cost of putting a network/telemetry sink behind the callback seam rather than inside `boyko_log` (§Refused), and G13c demonstrates it rather than leaving it as prose. |
| **E25** | **`OUT_LOCK` acquire times out** | The writer **steals**: it writes anyway, increments `OUT_STEALS`, and emits `boyko-W0110` once. An interleaved line is a legible defect; a hung process is not (Decision 9c). |
| **E31** | **A record is emitted while the runtime flag is off** *(new at the split, S13)* | The gate fails on a `.bss` byte that no initialiser has written: `CONTROL[T::ID]` reads `Level::Off == 0`. No lane is claimed, no buffer is touched, the clock is never calibrated, and the record does not exist. This is E1's mechanism promoted from "before boot" to "for the whole run" — the two are the same state, which is why no new machinery is needed for it. What it costs is one load and one predicted branch per surviving site, forever, and only the compile ceiling removes that. |

---

## Open questions

1. **STRUCK — `report!` no longer exists** *(S1)*. v3 asked whether `report!` should gain a
   schema-versioned TOML form. The measurement channel is the profiler's end to end and its output
   is an artifact, so the question moves to the profiling plan in a stronger form (the artifact *is*
   schema-versioned) and is not this plan's to ask.

   1b. **Retail diagnostics footprint — VALUES call, owner.** *(Collected, with the other owner
   calls, in `../SEAM.md`; kept here so a reader of this plan meets it where the numbers are.)* The
   joint retail figure is **`seam/joint-cost`'s to state** and this row no longer restates it or its
   operands. What it carried — 1.95 MiB, "this crate 1.16, the profiler 0.85" — is withdrawn on two
   counts. (i) **1.16 + 0.85 = 2.01, not 1.95**: the figure never equalled the sum of its own
   operands, in this revision or any earlier one. (ii) This crate's `shipping` half is **1.19 MiB**,
   re-summed from the `.bss` matrix's own rows in `logging/ring-and-statics` — not 1.16 (rev 3's) and
   not the 1.15 the owner's table currently sums, which is why that correction is flagged there with
   its consequence spelled out (`0.89 + 1.19 = 2.08 MiB`). **The VALUES question is unaffected and is
   the point of this row**, and the correction makes it larger rather than smaller: the joint retail
   figure, whatever it resolves to, stands against a profiling
   headline of "≤ 1 MiB retail" the owner may have read as the *whole* diagnostics budget. Reducing
   it means cutting one of: this crate's 32 × 16 KiB lanes (512 KiB), `SINK_OUT` (256 KiB), or the
   profiler's dynamic-zone arenas (96 KiB). How much does a shipped title pay for diagnostics?
   **S13 re-cuts what the number IS but not the question**: it is a reserved extent touched only by
   a player who asked for diagnostics, not a resident cost — so the VALUES call is about address
   space and about what a diagnostics-on run costs, not about what every shipped copy pays at idle.

   1c. **`shipping-min` semantics — SCOPE call, owner.** This plan's `ShippingMin` now has no
   resident *logging* thread (`SinkMode::Scheduled`, B8). The profiling plan's `Always` tier still
   writes a telemetry stream **synchronously on the dispatcher** in the same profile, so a title
   that chose `shipping-min` to avoid resident diagnostics still pays a per-window `write_all`.
   Keep, or make `shipping-min` also disable telemetry?

   1d. **How the enable flag arrives — SCOPE call, owner, new at the split** *(S13)*. Measured this
   session: `std::env::args` and `args_os` appear **zero** times across `crates/`, `src/` and
   `scripts/`, so "`--profile` / `--log=debug`" names a facility this workspace does not have. The
   two routes (an env var matching the 28 existing `BOYKO_*` switches, or a new argv parser in
   `boyko_app` that must be specified rather than assumed) and their costs are in `../SEAM.md`. The
   enable path is identical either way, so **no rung is blocked on the answer**.
2. **`--explain` delivery.** This plan embeds a one-line `summary` and requires
   `docs/diagnostics/<code>.md` (check 2). It does not add a `boyko-explain` binary. rustc's registry
   stays honest partly *because* three consumers read one table; we have two. Does the owner want the
   third?
3. **`.bss` budget.** Decision 3's matrix: **≈ 2.90 MiB** reserved for `dev` (v3 said ≈ 3.4; S3 cut
   lanes 128 → 80 and B2 added a 256 KiB handoff), **≈ 1.19 MiB** for `shipping` *(corrected from a
   carried 1.15: the matrix's `shipping` rows sum to 1 220 KiB, 40 KiB above the printed figure, and
   the derivation is written out in `logging/ring-and-statics`)*, demand-zero,
   resident a small fraction — and **zero** while the runtime flag is off, because no lane is claimed
   and no buffer is touched (S13). Acceptable, or should `dev` default to 64 × 8 KiB lanes? *(See
   also 1b: in isolation this is a small number; the joint total is `seam/joint-cost`'s.)*
4. **Sink thread in shipping builds.** One extra OS thread, idle-parking at 125 Hz — **and, after
   S13, one that does not exist at all until the flag turns it on**. **v2's inference here is
   struck** *(F3)*: it said "if P1 comes back `NOT RESOLVED` the thread is free", which is the
   reading of silence as proof that this document forbids everywhere else. `NOT RESOLVED` means
   **UNPROVEN**. The question to the owner is therefore a VALUES call and not a measurement: is one
   parked OS thread acceptable in a *diagnostics-on* shipped run, given that `ShippingMin` exists
   with **no resident logging thread** — paying instead a bounded per-frame drain on the frame thread
   and a stated hole around boot/shutdown (Decision 10, B8)? *(v3 offered `Manual` here, which had no
   consumer at all and was therefore not a real alternative.)*
5. **The `15xx` / `90xx` block split** is grandfathered and tidying is forbidden. Confirm —
   renumbering breaks the book, the `#[should_panic]` assertions and the never-reuse rule at once.
6. **The one-line RHI fix for sync-validation** (`pLayerName = "VK_LAYER_KHRONOS_validation"`) is
   deliberately not in this plan. It has a large blast radius: sync-validation coming alive would
   surface real hazards and could turn every golden run red. Pull it into L7, or keep it a separate
   RHI item coded by `E2101`? **Note that G7's negative leg is re-cut around its absence** (F2), so
   answering this does not block L7.
7. **`OUT_LOCK`'s registration is now moot** *(resolved by F9)*. v2 asked the owner to confirm a
   `docs/HOT-PATH-EXCEPTIONS.md` entry. That entry is **not implementable**:
   `scripts/check_hotpath_exceptions.py` matches rows against `#[allow(clippy::disallowed_types)]`
   counts per file, and an `AtomicU64` carries none, so the row would red CI. The question that
   remains is narrower: **confirm the steal-on-timeout trade** (Decision 9c) — a possibly-interleaved
   line instead of a possible hang — rather than an attempt to make the channel lock-free, which is
   strictly more machinery for a path that is cold by construction.
8. **Dynamic band size.** 32 slots (IDs 224..=255). A modding-heavy title could want more, but every
   extra slot comes out of the 256-target space that `CONTROL`, the sink filters (`[u64; 4]`) and
   `TARGET_STATS` are all sized by; past 256 those three arrays become two-level structures.
   **Recommendation: ship 32 and treat "more than 32 data-defined categories" as a signal that the
   taxonomy belongs in source.** VALUES call.
9. **Is `log-sampling` default-on?** Decided by G10d, not by this document. Flagged so the answer is
   recorded rather than absorbed.
10. **Does the engine profile want rotation?** `Rotation::NONE` is kept as the engine default so a
    bench cannot silently discard its own beginning. A long editor session would want rotation.
    **Recommendation: `dev` keeps `NONE`; a future `editor` profile gets rotation.** SCOPE call.
11. **Telemetry payload shape.** `LogCensus` gives a game per-target counts; the binary log gives it
    everything. What it does *not* give is a compact per-session summary suitable for upload (a few
    hundred bytes). That is a game-side reduction over `LogCensus`, deliberately not designed here —
    but if the owner wants a canonical shape shipped, it is a small additive rung after L16.
12. **Should `boyko_ui`'s console (L9) live in this plan or the UI plan?** L16 now fixes the entire
    contract it consumes (`since`, `RingFilter`, `skipped`, `LogCensus`, the control-epoch counter),
    so L9 is a pure UI rung with no logging design left in it. Recommend moving it wholly to the UI
    plan and deleting L9 from this ladder.

---

## Checklist

**Structure** — goal in perf+functional terms, both audiences ✔ · concrete targets with named
red-state controls ✔ · every decision justified via perf/cache/parallelism ✔ · alternatives rejected
with reasons ✔ · trade-offs listed ✔ · the five audience conflicts named, decided, and costed for
the losing side ✔ · eight asks explicitly refused with reasons ✔ · **the joint
(both-subsystems-present) cost stated where the reader meets each number, not only in a table** ✔ ·
**the flag-off cost stated as a three-row table (memory / boot work / per-site instruction) rather
than as the word "zero", with the row that cannot reach zero named** ✔ *(S13)*

**Data structures** — every field typed + commented ✔ · `repr`/`align`/`packed`/`transparent` where
it matters ✔ · three-partition cache-line split ✔ · sizes pinned by `const _: () = assert!`,
**including the `COMMIT_GRANULE` divisibility pin that v2 violated** ✔ · **`Send`/`Sync` pinned by a
`const _` `assert_send_sync`, because `VmColumn` is neither** ✔ · false-sharing padding specified for
`LogLane`, `HandoffRing`, `RateSlot`, `DynSlot`, `TargetStatCell`, `SinkSlot` ✔ · **the sink→ECS
handoff is a specified structure with layout, capacity, ordering, overflow and a budget row** ✔ ·
producer working set ≤ 4 lines in isolation, **7-8 jointly, both stated** ✔

**API** — minimal ✔ · no internal types in signatures ✔ · lifetimes trivial ✔ · no `dyn` anywhere
(the callback seam is an `extern "C" fn` + ctx) ✔ · generics where specialisation is needed
(`emit_impl<A: LogArgs>`, the `LogPod` blanket) ✔ · **no public type can hold an out-of-range index**
✔ · **no encode path reads an uninitialised byte** ✔

**Multithreading** — model explicit ✔ · every atomic's ordering stated, including the new data ✔ ·
**every wait bounded, including `OUT_LOCK`'s** ✔ · the one sync point short-circuited when no
consumer exists ✔ · **the consumer role is CAS'd directly (`DRAIN_OWNER`), not inferred from a state
that merely correlates with it** ✔ · `Send`/`Sync` consistent **and argued for the two types that
cannot derive it** ✔ · race-freedom argued, including the single-thread re-entrant case, `LogPod` and
the handoff ✔

**Correctness** — 31 edge cases ✔ *(E31 added at the split)* · **the admission arithmetic proved by
induction and proptested over raw integers** ✔ · session-scale integer audit with a per-quantity
table **covering `seq_lo` and every `BinarySink` width** ✔ · lane identity single-sourced ✔ · drop
order (E16) ✔ · `unsafe` invariants in the `Sync` SAFETY blocks (clauses 1-4 incl. 1c-1f, plus the
handoff's four) and per algorithm ✔

**Integration** — machine-generated ledger, not a hand table ✔ · census arithmetic reconciled and its
denominator named (**≤ 78** after S1) ✔ · API changes explicit ✔ · `Arena`/`ComponentPool`/`UnitId`
untouched ✔ · **this plan writes no stdout at all** ✔ · 22 rungs plus 2 joint rungs, each landing
green and committing alone, each with a "must NOT move" column ✔ · **every cross-plan precondition
named with the rung it waits for** ✔

**Validation** — 33 live tests ✔ *(the ladder's list is numbered to 34; entry 16 is `DELETED (S1)`
and its number is deliberately not reused, so a reader of an older review can still find what
happened to it — the count is of LIVE tests, not of the highest number)* · 6 property families ✔ · 14 benches with in-sitting controls **and a
`config_tag`** ✔ *(the 14th is GJ1's, added at the split)* · `debug_assert!` **and release-live**
invariant lists ✔ · every gate has a named red state **and an explicit "what this gate CANNOT claim"**
✔ · three gates (G8d, G10d, G12c) can **revert their own rung** ✔ · **one gate (G17) exists to red on
a defect this document previously shipped, at two named fill levels because one of them is vacuous**
✔ · **three v3 gate legs deleted or relocated for being unable to go red** (G17c, G4d, the
`vk-validation` census row) ✔ · **one gate (GJ1) whose control leg can invalidate the whole
measurement rather than letting it report a number** ✔

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
| **M24** | `report!` and the sink share one fd unsynchronised | **FOLDED** | §Decision 9/9b — both take `OUT_LOCK`; console sink → stderr; S7's stderr line-integrity test at L3-gate runs the **concurrent** state (`logging/ladder`, mandatory-test entry 16). *(v4: `report!` is deleted by S1 and test 16 with it; the fold stands on Decision 9/9b, because `OUT_LOCK` survives S1 with seven remaining callers.)* |
| **M25** | L7 control is one this machine cannot show | **FOLDED** | §L7-gate — forced hazard replaced by an ordinary validation error (`mip_levels: 12`) with the baseline-19 accounted; `E2101` made a two-sided gate |
| **M26** | 129th-thread loss is designed in; test 5 one-sided | **FOLDED** | §Decision 5 / E6 — synchronous fallback for `Warn`/`Error` on claim failure; test 8 asserts the fallback delivered and states reclaim is eventual |
| **N27** | `option_env!` evaluation site unspecified | **FOLDED** | §Decision 2 — `GLOBAL_CEILING` is a `const` in `boyko_log`, referenced as `$crate::GLOBAL_CEILING`; never expanded in a caller crate |
| **N28** | `AtomicPtr<u8>` loses the `&str` length | **FOLDED** | §Data structures — `AtomicPtr<TargetInfo>` publishes name+len with one pointer |
| **N29** | `MAX_RECORD_BYTES` reachable from safe code | **FOLDED** | §Data structures / E3 — raised to 2048 **and** checked at runtime in every profile; dropped + `TOO_LARGE` + counted, not a debug panic |
| **N30** | `W0101` has no showable red state | **FOLDED** | §Decision 11 — written down as uncontrolled and listed in `tests/untested_codes.txt` with that reason. *(v4: the code is **deleted** outright by S4 — see the seam table in `../SEAM.md`.)* |
| **N31** | `decode` monomorphisation count is an unmeasured claim | **FOLDED** | §L1-gate G5 — distinct-`decode`-symbol count asserted against an upper bound; the prose claim is removed |
| **N32** | Header duplicates `code`/`level` | **FOLDED** | §Data structures — header shrunk 24 → **20 B packed**; `code`, `level` and `lane` removed (site holds two, the drain knows the third) |

**Refuted: none** (of B1-N32). Every one was either a defect in v1's own pseudocode, a gate that
could not fail, or a claim v1 made without a control. Two of these folds were themselves defective
and are re-folded below: **M13** (F7 — the fold did not run), **M25** (F2 — the fold installed a
second unshowable control). **M11** and **M24** are amended (F10, F13).

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
| **F13** | M24's "verified" claim misdescribes its own consumer | **FOLDED** | §Invariant 2 + §Compatibility *(v4 also cited **test 16**; that leg is struck — test 16 is deleted by S1 and its number is not reused. The fold rests on §Invariant 2 + §Compatibility, which S1 does not touch.)* — `vg_occ_split_timing.rs` reads `stdout`/`stderr` as **two separate buffers** and concatenates them in-process; the merged consumer is `golden.ps1:196-202` via `cmd /c`. The real hazard is intra-stdout (F17). **CITATION CORRECTED at the split**: v4 cites `vg_occ_split_timing.rs:1115-1117`, which today is a doc comment on `per_frame_us`. The actual site is **`:1135-1136`** (`String::from_utf8_lossy(&out.stdout)` then `push_str(…&out.stderr)`), with the descriptive comment at `:708`. The finding and the fold are unaffected; only the line numbers were stale |
| **F14** | Machine-consumer inventory incomplete; a third gate's entire input uncited | **FOLDED** | §Invariant 2 — measured inventory: **31 files** reference `[vk-validation]`, **16** reference `VB-P1d`. `vb_bench_query_validation.rs:116-118`'s byte-exact constant (trailing space) is now pinned by test 17 |
| **F15** | `TargetId(pub u16)` vs `MAX_TARGETS` with a debug-only bound | **FOLDED** | §Decision 15 + §Data structures — private field, closed constructor set, `get_unchecked` sound with a SAFETY clause naming it. `INVALID` deleted in favour of `Option<TargetId>`; E19 rewritten; the `debug_assert` removed as vacuous |
| **F16** | Principle 0 exception neither named nor argued | **FOLDED** | §Invariant 7 — claimed in writing on dependency inversion, with the cost (no `Query`, no change detection) stated and the control-epoch counter named as the mitigation |
| **F17** | Buffered-stdout race between `report!` and the surviving `println!` sites | **FOLDED** | §Decision 9 — `report!` writes **through `stdout()`'s own `LineWriter`**, not a raw handle; cost (one memcpy + one flush) stated. Test 16's red state is the raw-handle variant. *(v4: `report!` is deleted by S1; the finding's fold is historical.)* |
| **F18** | Census arithmetic: 83 vs the ledger's 98, over a corpus containing non-sites | **FOLDED** | §Goal + §Integration ledger — 179/36 reproduced by one command; 98 named as the occurrence remainder; the **walker's site count** named as the migration denominator so comment mentions can never be driven into the allowlist |
| **F19** | G1 is scheduled at L0-gate but depends on L1 | **FOLDED** | §Implementation plan — G1 moves to **L1-gate**; L0-gate keeps G4 and G2, both of which have real red states at L0 |
| **F20** | L2-gate reds (or goes vacuous) on the grandfathered corpus | **FOLDED** | §Decision 6 — registry rows carry `Live` / `Pending(rung)`; check 3 (Live ⇒ ≥1 emitter), check 3b (Pending ⇒ 0 emitters), check 3c (`Pending == 0`, armed at L8c). L2 commits alone; a `Pending` row cannot rot silently |
| **F21** | `LANE_ARRAY_LEN = 0` vs `LANES[i]` indexing in Algorithms B | **FOLDED** | §Decision 3 + §Algorithms B — the claim scan is written over `LANES.iter()`; zero-length is zero iterations |
| **F22** | `dsp!`'s described form does not borrow-check | **FOLDED** | §Decision 13 — the `DspBuf<N>` by-value-temporary form is written out, with the end-of-statement lifetime argument and the 256-byte cost in the trade-off |
| **F23** | `Lane`'s SAFETY block does not cover `read_cached` / `write_cached` | **FOLDED** | §Data structures SAFETY clauses **1e** and **1f** — single-role ownership plus the staleness argument in both directions |
| **F24** | `W0102` is unbudgeted: ~16 000 sink-generated records/s during a drop storm | **FOLDED** | §Decision 5 + §Algorithms C — **one aggregated `W0102` per drain** (125/s) carrying `lanes_affected`/`records`/`bytes` and, since v4, the `LossClass` breakdown in place of v3's `SATURATED` flag (S8); per-lane detail moves to the polled census |
| **F25** | `STAGE_BYTES`'s backing store unspecified | **FOLDED** | §Data structures — `STAGE`, `SITE_DICT`, `SINK_OUT` are `.bss` statics, counted in Decision 3's budget matrix, with the "no `Vec`/`Box` in a *signature*" narrowness called out |
| **F26** | Zero-alloc test 3 proves only the steady state (TLS dtor registration) | **FOLDED** | §Metrics test 3 — three legs (steady 0, first-emit ≤ 1 with the number recorded, monotonicity over 1000 emits). *(v4: S3 deletes the `Drop` guard, so the first-emit leg becomes `== 0` exactly.)* |

### Refuted, with the evidence

| # | Claim | Refutation |
|---|---|---|
| **F5 (count)** | "28 occurrences of `boyko-B1802`, every one inside a `/// # Panics` doc comment" | **Partially refuted.** `crates/boyko_ecs/src/ecs/core/app/app.rs` contains **24** occurrences: 18 in doc comments, 1 panic-message string (`:867`), 5 `#[should_panic(expected=)]` inside the in-`src` `#[cfg(test)]` module (`:898`-`:939`). The *finding* is correct and folded; the *count* is not, and the fix is specified against the measured shape (the `#[cfg(test)]` region also has to be stripped, which the review's framing did not surface) |
| **F12 (part)** | "`eprintln!` currently takes `stderr()`'s own lock, so the replacement … is a regression against M24's own concern" | **Accepted as to `debug.rs`** — and the conclusion is stronger than the review's: the whole edit is withdrawn, so the concern cannot arise. But the *general* claim that `stderr()`'s lock made v2 safe is not why `report!` needed fixing: `report!` writes **stdout**, where the hazard is the `LineWriter`, not the lock (F17). The two are separate defects and are fixed separately |
| **F16 (framing)** | "`boyko_log`'s statics are almost certainly correct but Principle 0's named exceptions do not cover them" | **Accepted, and the exception is claimed** — but not as a formality. The argument is *load-bearing*: it is the same argument that forbids `CONTROL` from being an ECS column (§Refused), and it is what makes the crate usable from a driver callback and a panic hook. Recording it as a bare exception would lose that |

**One finding is deliberately NOT folded as written:** the review's open question 5 asks "`LogLine`
must be `Copy` and its size must divide 64 KiB. Which size — 8 or 16?" **16**, with the fields listed
in `logging/game-facing-surface`. 8 bytes cannot carry `start` + `len` + `code` + `level` + `target` without
dropping the sequence number that Decision 26's reader cursor needs.

---

## Findings disposition (v3 → v4 review, verdict REJECTED)

| # | Finding | Disposition | Where in v4 |
|---|---|---|---|
| **B1** | `LogRing`/`LogCensus` cannot be `Resource`s — `VmColumn` is `!Send + !Sync` | **FOLDED (correctness bug)** | §Data structures, "B1" block — verified against `vm_column.rs:70` and `resource.rs:42`, both re-read. A SEND10-shaped `unsafe impl` with a **named holder set**: `ResMut` only in `log_drain_system`, `Res` readers via the scheduler's exclusivity, and — the load-bearing clause — **the sink thread never touches either type**, because B2 gives it `ECS_HANDOFF` instead. Quotes `vm_column.rs:73-77`'s own invariant list for the columns, and notes that `LogPlugin::build` materialises before the schedule runs so the write-once `base` never races. Pinned by `const _ { assert_send_sync::<LogRing>() … }` — F7's treatment applied to `Send`/`Sync` |
| **B2** | The sink→ECS "handoff ring" is referenced three times and defined nowhere | **FOLDED** | §Decision 26 + §Data structures + §Algorithms C — `ECS_HANDOFF`, a first-class `HandoffRing` with **the same shape and wrap rule as `LogLane`** (no new protocol), plus type, capacity (256 KiB / 64 KiB), ordering rows in §Multithreading, a `.bss` budget row in Decision 3, overflow accounting (`LossClass::Sink`, `W0117`, `lossy`), a four-clause SAFETY block, and a presence rule (absent when `ecs_ring` is off). G15 gains leg (c) |
| **B3** | G17 leg (c) cannot go red — the F6 overrun is intra-lane by construction | **FOLDED** | §Gates G17 — the neighbouring-canary claim is **deleted** (and relocated to test 6's wrap proptest, where an off-by-one *can* cross a lane). Leg (c) becomes "pre-seeded **undrained** records are byte-unmodified after the refused emit", **at a second, explicitly named fill level** (`used > CAPACITY − need`), because at the reserve-boundary fill the broken arithmetic writes into genuinely free space and leg (c) would be vacuous there too. The arithmetic for both fills is worked out in the gate row |
| **B4** | G4 leg (d) measures argument evaluations while asserting a delivered count, and needs L12's mechanism at L0 | **FOLDED** | §L0-gate (leg deleted) + §Gates **G10e** at L12-gate (leg re-created with both numbers: **1000 evaluations AND 500 delivered**) + §Algorithms A (**LANE moved ahead of SAMPLE**, so no lane-indexed state is touched before the lane exists, and E6's unlaned thread skips sampling rather than indexing a row it has no claim to). RATE keeps its position, since it is code- and site-indexed, so a suppressed record still costs no lane claim |
| **B5** | The crash-drain CAS treats `Manual` as quiescent; it is not | **FOLDED (correctness bug)** | §Decision 24 — `DRAIN_OWNER`, an `AtomicU64` role token CAS'd identically by all **four** consumers (sink thread, `drain()`, `Scheduled`, crash). `SINK_STATE` loses its exclusivity job and `CrashDraining`. `drain()` returns `DrainResult::Busy` rather than asserting, because a second manual caller is a user error and a `debug_assert` is silent in release. §Gates **G14 gains leg (c)** — panic with a manual drain held open by a barrier — with the red state being v3's exact CAS. E26/E27 added; the `DRAIN_OWNER` CAS is **release-live** |
| **B6** | Registry check 4 cannot be green at L2 — its corpus names ~25 unregistered codes | **FOLDED, with the measurement redone** | §Decision 6 — TEXT becomes an **explicit directory list excluding `docs/archive/**`**; a `Historical` row status is defined for any future re-inclusion; `docs/diagnostics/B9004.md` and `B9005.md` are named as L2 line items (both exist in source and in **no** document); the block map's "occupied today" column is seeded from the measured 9 distinct source codes; `92xx` is reserved at L2 with **18** `Pending` rows (`W9201`..`W9218`; this row said 17 and the count is adjudicated in `seam/diagnostic-code-space`) and its own measured free-band note. **Measured**: 75 occurrences / 13 files, `docs/archive/**` **29** (not 27), and a case the review and the addendum both missed — code **W9003 at `docs/archive/PHASE-15-PLAN.md:471`**, which is *also* this document's own check-4 red-state example, so v3 would have red **on itself**. Every prefixed literal in v4 names a code this plan registers; unregistered codes are named bare |
| **B7** | The walker's `#[cfg(test)]` rule cannot exclude the files the ledger says it excludes | **FOLDED** | §Decision 6 — the rule is **cross-file** and specified without a Rust parser: a pre-pass collects `#[cfg(test)]` + optional `#[path]` + `mod NAME;` declarations and marks the resolved file test-only. Verified per file: `compute/tests.rs` 16 within-file; `brick/tests.rs` gated at `brick.rs:1829-1830`; `colored_tests.rs` gated at **`colored.rs:3198-3200` through a `#[path]`** — which the review noted was ungated but did not locate, and which is why the rule must follow `#[path]` too. `#[cfg(any(test, …))]` is treated as test-only **and listed by name** in the walker's report. The `179 − 58 − 23` arithmetic is re-derived against the rule that will run |
| **B8** | `shipping-min`'s crash file structurally contains the session's beginning | **FOLDED** | §Decision 10 (new `SinkMode::Scheduled`, with **both** of v3's rejection grounds addressed — one answered by `DRAIN_OWNER`, the other **conceded and written down as the profile's cost**), §Decision 25 (`ShippingMin` uses it), §Decision 5 (why the answer is not overrun-oldest), C4, E22. **The retained window is restated in RECORDS**: ≈ 13 100 across 32 lanes, ≈ 410 per lane — and under `Scheduled` that ceiling is never approached, so the crash file holds records adjacent to the crash |
| **B9** | The sync route, the exhaustion fallback and E22's mitigation all write a stderr a shipped title does not have | **FOLDED, with one premise corrected** | §Decision 9c "the durable fan-out" — `write_oracle_line` writes **every** configured synchronous destination, including the crash file. G18 gains leg (c). Decision 20's cost is restated: **~200 ns is uncontended and console-only**, the contended bound is `OUT_LOCK`'s 50 ms steal deadline, and "durable-on-write" now means a `write_all`, with `fsync` opt-in (`sync_durable`) at ~0.1-10 ms. **Premise corrected**: `grep -rn windows_subsystem crates src` returns nothing, so stderr is a valid handle on this tree *today*; the invalid-handle case is a future shipping configuration. The **durability** defect was real in every profile including `dev`, which is the half that carries the fix |
| **B10** | `LogPod`'s blanket encode copies padding; the sink materialises `&[u8]` over uninitialised bytes | **FOLDED (correctness bug)** | §Decision 19b — the blanket `copy_nonoverlapping` is **deleted**. The trait requires `unsafe fn encode_pod(&self, dst: *mut u8)`; the derive generates it **field-by-field through `LogValue`** (which it already required), rejects dynamic-length fields so the sum stays a `const`, and emits `const _: () = assert!(POD_LEN == Σ field lengths)`. `POD_LEN` is no longer `size_of::<Self>()`. **G9b's subject changes** to the padded-encode red, and the `debug_assert!` list loses `POD_LEN == size_of::<Self>()` — the assertion that made the UB look checked |
| **M1** | The census line that fixes F10 is uncomputable after F11 moved the latch per-site | **FOLDED** | §Decision 8 point 2 + §Data structures `OnceSite` — an intrusive, insert-only `ONCE_SITES` list whose **nodes are the per-site statics the macro already expands**; pushed by a `#[cold]` CAS on the site's single fire, so nothing is added to the steady-state path. The census prints **one row per fired site** (three rows for `W2102`, which is the F11 case), and a never-fired site is **absent**, which is itself the datum. **`RateSlot::fired` is deleted** as dead. Test 31 extended |
| **M2** | The session-scale audit omits `seq_lo` and every `BinarySink` quantity | **FOLDED** | §Decision 21 — six ✚-marked rows added: `seq_lo` **with its reconstruction rule** (`seq = ring.seq − (ring.seq_lo ⊖ line.seq_lo)`, unambiguous because the ring holds ≪ 2³¹ lines, so the high half is never stored and never needed), `tsc_delta` **with the anchor cadence its 1.4 s span forces**, `site_id` against the 4096-entry dict (with `W0116` + inline site records on a full table), `len`/`flags`, file offsets and the rotation counter, and `clock_epoch_lo`. Decision 22 states that the widths are pinned **there**, not deferred to `docs/LOG-BINARY-FORMAT.md`. Test 20 and a property family extended |
| **M3** | Downstream code-index exhaustion has no defined behaviour | **FOLDED** | §Decision 19 — `CODE_IDX_EXHAUSTED`, a reserved sentinel that is **never an aliased index**; the record is **still delivered** with `Every` semantics; `boyko-E0115` once; `LogStats.codes_unindexed` thereafter. **G9 gains an exhaustion leg** whose red state is a modulo-wrapping mint that shares a `RateSlot`. E30 added |
| **M4** | The census's own showcase row is permanently `UNPROVEN` by construction | **FOLDED** | §sync-validation confrontation — **the `vk-validation` census row and target are deleted**, with the reason written where the row was: Decision 9b guarantees no record can ever reach that target, so the row could never move and invited the opposite misreading. The census is illustrated by a target records actually reach; `E2101` + G7 remain the liveness claim. Test 17 leg (c) loses its census clause. The review's alternative (count messenger callbacks) is rejected in writing, because it requires editing the byte-frozen `debug.rs:114` |

### Refuted, with the evidence

| # | Claim | Refutation |
|---|---|---|
| **B6 (archive count)** | "`docs/archive/*` (10 files) — 27" | **Refuted as to the number.** Re-measured with `grep -roE 'boyko-[BEW][0-9]{4}' docs --include='*.md'`: `docs/archive/**` contributes **29** occurrences across 10 files, and the four-way split is 41 / 29 / 3 / 2 = **75**, which is the addendum's own stated total (its 41 + 27 + 3 + 2 sums to 73). The **finding is correct and folded**; the count is not, and the fix is written against the measured composition |
| **B6 (missed code)** | The archive-only dead codes are "`B9000` and `B9003`" | **Incomplete, and the omission matters.** The archive's distinct set is `B0002 B1801 B9000 B9001 B9002 B9003 B9101 W1501 W9003`, so the codes with **no emitter and no current doc** are `B9000`, `B9003` **and `W9003`**. `W9003` is the one that bites: v3's check-4 row used its **prefixed literal** as the *illustrative red state*, and check 4 scans this file — so v3's check 4 would have red on that very document, permanently, against a real archive code. Both facts are folded (the corpus excludes `docs/archive/**`, and unregistered codes are named bare in this corpus); neither the review nor the addendum had the second |
| **B9 (premise)** | "a shipped windowed Win32 title does not have [stderr]" / "in any windowed Win32 title, where stderr is an invalid handle" | **Refuted as to the tree, accepted as to the design.** `grep -rn windows_subsystem crates src` returns **nothing**: every binary here is a console-subsystem binary and stderr is a valid handle today, so the invalid-handle failure is a *future* configuration rather than a present one. The finding survives the correction with room to spare on its other leg — stderr is neither the log file nor `fsync`ed, so "durable-on-write" was false in **every** profile including `dev`, and `shipping`/`shipping-min` configure no console sink at all, so the bytes go nowhere collected. The fix is unchanged; only the argument for it is |
| **S5 (`E0110`)** | The seam record specifies "`E0110` on a ninth [`PRE_FLUSH` registration]" | **Refuted as to the number.** `W0110` is already `OUT_LOCK`'s steal code. `DIAGNOSTICS` is dense with `index == code_idx`, and registry check 1 asserts numbers **strictly increasing**, which is also a `const _: () = assert!` — two rows numbered 110 would not compile, whatever their class letters. The mechanism is adopted verbatim; the code is **`E0118`**, the next free slot in the `01xx` band. Recorded in the seam table (`../SEAM.md`) so the divergence is not read as a transcription error |

**Not refused, and worth saying so:** every one of B1-B10 and M1-M4 is folded. Four of them (B1, B5,
B8, B10) are correctness bugs of the same family this campaign keeps finding — a proof that holds
for the case the author had in mind and not for the case the code admits. Three (B3, B4, M4) are
gates that could not go red, found **inside** the gates written to stop gates that cannot go red.

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

The `[vk-validation]` channel (**not edited at all** — v2's one edit is withdrawn too) · Decision 7
(`Warn`/`Error` MUST carry a code — no exception for games, mods or scripts) · Decision 13's
structural re-entrancy exclusion (`LogPod::fmt_pod` runs on the sink, asserted by test 24) ·
Decision 12's core (no handle, bounded waits, detach-not-join) · Decision 1's deferred-format core
and the **20 B** packed header · the eight registry checks · Decision 3's off-build · the
`.bss`/`Off == 0` regime · the SPSC lane ring and its SAFETY clauses · gates G1-G7 and their limits ·
every migration-ledger disposition except the measurement rows.

*(Four items on v3's list of "did not change" **did** change in v4, and are listed here so a reader
of both versions is not misled: **`report!`** is deleted (S1); **`W0101`** is deleted (S4); the
registry's **corpus rules** are re-cut (B6, B7); and Decision 12 gains `PRE_FLUSH` and
`sink_can_accept()` (S5). None of the four was touched by the scope extension — they were moved by
the seam and by the third-pass review.)*

*(One further item moved after v4, at the corpus split: Decision 12's `boot()` no longer spawns the
sink thread or installs the panic hook — `enable()` does, so a flag-off run has neither. The
mechanisms are unchanged; only their call site is. S13, `../SEAM.md`.)*

### Refused, with the reason recorded so it is not re-derived

| Ask | Reason |
|---|---|
| **Network sink inside `boyko_log`** | Blocking, retries, back-pressure — the three properties this crate refuses. `CallbackSink` on the sink thread is the seam; the game owns the policy because only the game knows whether stalling or dropping is worse for it. G13c demonstrates the stall cost rather than leaving it as prose |
| **Cross-process shared-memory ring** | The record carries a process-local `*const LogSite`. Supported instead: per-process files, one `SessionId`, `logdec --merge`, with the clock-agreement bound **printed** |
| **Gameplay decisions on log counters** | Lower bounds under drop, schedule-dependent, non-deterministic across machines ⇒ breaks replay. Display and telemetry are supported (`LogCensus.lossy` is the bit that keeps a UI honest); gameplay counters belong in the game's own components — Principle 0's answer |
| **Data/script-defined *codes*** | A code is a promise of a documented page; a data-defined code cannot have one. Supported instead: one class code per game subsystem, plus dynamic *targets* |
| **`CONTROL` as an ECS column with `EnableTag`** | Dependency cycle · no `World` at boot, in a panic hook or in a driver callback · ~10-30 ns per emit against a 1-instruction budget. The rule's substance (capability structural, state a bit) is applied at the layer that can afford it, and the exact cost of the refusal (no `Query`, no change detection) is stated with the control-epoch counter as the mitigation |
| **Per-entity log storage in `ComponentPool`** | Logging is not per-entity data; `UnitId`'s two-level addressing buys nothing for a byte ring |
| **`Box<dyn Sink>` plugins** | `extern "C" fn(&FormattedRecord, *mut ())` + ctx crosses a dylib boundary with no vtable and no allocation |
| **A second sink thread for the binary sink** | Two consumers on one lane set is what the `LogLane` SAFETY block forbids — and after B5 that is enforced by a `DRAIN_OWNER` CAS a second thread would simply lose. Sinks fan out **inside** one drain, so text + binary + crash cost one pass |
| **Reaching the file sink's handle from `write_oracle_line`** | That handle is owned by the consumer role; writing it synchronously from an arbitrary thread is a second consumer of the sink's own state. The durable fan-out therefore targets the **crash** handle, which is opened once on the enable path and appended to under `OUT_LOCK` — one destination, one lock, no second consumer (B9) |
| **Growing the ring to answer "as much data as possible"** | Enlarging the ring moves the loss point; it does not raise the throughput ceiling, which is `core::fmt` on the sink. The answer is to **not format** (`BinarySink`) — and that claim ships with a revert clause (G12c) rather than as an assertion |
| **Hoisting or hot-patching away the per-site runtime check** *(new at the split, S13)* | Hoisting a global read out of a loop across an opaque call is something the compiler *may* do, not something it must — a hope rather than a mechanism. Patching call sites at enable time writes executable pages at run time, a new capability class in a crate whose emission path is *defined* by having no allocation, no lock and no syscall. The honest statement is the one in the S13 cost table: the branch stays, and only the compile ceiling deletes it |

### The one thing this plan says plainly is a bad idea

**Do not make the logger a substitute for a missing source.** This is not a hypothetical:
sync-validation is **dead on this machine** — a genuine missed barrier produced 19 messages (the
baseline), zero `SYNC-HAZARD`, and a byte-identical golden, twice. A logger is a transport. It
changes where a message goes and has no opinion on whether the message exists. Routing a dead
channel through a prettier pipe makes the deadness *harder* to see, which is why v1's migration is
withdrawn and why the census reports `UNPROVEN`, never `clean`.

The same reasoning bounds the game-facing ask. A logger cannot tell a game why its frame hitched if
nothing measures frames; it cannot tell a player's support agent what went wrong in a crash that did
not unwind; and it cannot make a sampled capture representative. Each of those is written into a
gate's "cannot claim" column rather than left for a reader to discover at the moment they need it to
be true.
