# Profiling — statistics discipline and the contrast API

<!-- CONTRACT
provides: profiling/statistics-discipline
provides: profiling/contrast-api
assumes:  profiling/budgets-and-invariants
assumes:  profiling/store-and-fold
assumes:  profiling/gpu-zone-seam
assumes:  substrate/clock-source
assumes:  seam/landing-order
assumes:  seam/vocabulary
-->

**Carved from** `docs/PROFILING-SYSTEM-PLAN.md` (rev 4) — §Measured facts from VG R3 P4-6,
§Statistics discipline, §D7, §D10, §D11, §D11a, §D13, the statistics/contrast blocks of §Data
structures and §Public API, and §Algorithms A4, A5, A6. Diff against that file until it is retired.

**Why this file exists at all.** A previous campaign in this repository shipped a band that measured
**DRIFT rather than RESOLUTION** and produced a **false `RESOLVED`**. It took four sittings to find,
and every one of the four failures was a defect in the *measurement*, not in the engine. Everything
below is that history turned into properties of an API — because advice in a paragraph is what
failed the first time.

---

## The seven MEASURED facts from VG R3 P4-6 (`cf2d367`)

P4-6 needed four sittings; **every failure was a defect in the measurement, not in the engine**. A
game-facing profiler will hit all seven constantly, which is why they are carried here in full rather
than summarised into rules.

1. **Two adjacent `BOTTOM_OF_PIPE` stamps cannot establish a strict order** — they resolve on the
   same tick. This kills rev 2's `__gpu_null` "quantum probe" (F6): its measured value on this box is
   **0**, every time. (`profiling/gpu-zone-seam` owns the deletion; this file owns the consequence
   for the band.)
2. **Equal timestamps cannot license a conclusion about RECORD ORDER.** Record order is a *host*
   property and must be witnessed host-side — `CommandWitness::first_pair_of`
   (`profiling/gpu-zone-seam`). No claim about record order in this system reads a timestamp.
3. **`median(off) + median(dur) ≠ median(off + dur)`** — composing medians crossed a true inequality
   by **144-240 ns** on a real reading.
4. **A zero twin whose expected value is exactly zero measures DRIFT, not RESOLUTION.** A0/A1 were
   the same configuration on a serialized deterministic GPU; the twin came back **exactly 0 on all
   ten passes**, and the verdict rule silently collapsed from *"clears the noise"* to *"is nonzero"*,
   reporting a **false RESOLVED**. This is the defect this whole file exists to prevent recurring.
5. The fix: **`band = max(floor, twin)`**, where `floor` is the propagated standard error of every
   median a reading is built from, sub-floored at the **measured** lattice quantum. In the tree:
   `crates/boyko_app/tests/vg_occ_split_timing.rs:1034` (`twin_term` → `band_of(&self.zero)`) for the
   twin term, `:867-871` (`resolution_of`) for the per-median resolution and `:883-885`
   (`floor_over`) for its sum, `:887-910` (`measured_quantum_ns`) for the quantum.
6. **The lattice is measured per sitting, never written down.** `timestampPeriod` is **1.0 ns** on
   this vendor and is the tick→ns **SCALE, not the counter increment** (`vg_occ_split_timing.rs:893-895`);
   flooring a band at it *"would satisfy every arithmetic check while silencing the alarm"*. The
   odd-budget sitting measured **32 ns**.
7. **An even sample budget puts medians off the lattice.** `DEFAULT_BENCH_FRAMES = 221`
   (`vg_occ_split_timing.rs:322`) is odd *deliberately* (`:315-321`): `vb_bench_stats_ns` returns
   `0.5 × (sorted[n/2 − 1] + sorted[n/2])` for an even `n`, so *"every published median was the MEAN
   OF TWO SAMPLES: a value no frame had, sitting half a tick off the timestamp lattice."* Removing
   the bias also removed — unplanned — the twin's degeneracy. **Consequence for this plan:
   `WINDOW` becomes 121, not 120.**

### Fact 6's "in-tree doc-rot" is REPAIRED IN THE TREE, and the repair supplies the mechanism

Rev 4 recorded, as known and deliberately unresolved, that *"two prose sites in that same file still
say 16 ns (`:138`, `:881`)"* while the odd-budget sitting measured 32. **That is no longer true of
HEAD, and it was checked rather than inherited.** `16 ns` occurs exactly twice in
`vg_occ_split_timing.rs` today, at `:141` and `:896`, and **both are explicit retractions**:

> *"An earlier text said 16 ns; that GCD was taken over medians from an EVEN budget, each the mean of
> two middle samples, so it could legitimately read `q/2`."* — `:896`

`:138` says the **opposite** of what rev 4 attributed to it — *"⚠️ The lattice is **measured**
(`measured_quantum_ns`) and is **not** `VkPhysicalDeviceLimits::timestampPeriod`"*.

This is carried as a **correction, not a softening**. The discrepancy was real; its cause is now
recorded in the tree, and the cause is *fact 7's mechanism* — an even budget makes each median the
mean of two middle samples, which can legitimately land on `q/2`. So the two facts are one fact seen
twice, and the rule they force is unchanged and strengthened: **the quantum is computed at run time
by `measured_quantum_ns` and never hard-coded**, and **every reduced window has an odd sample
count**. What must NOT be carried forward is rev 4's framing that the tree still disagrees with
itself; a later reader who "fixed" the tree on that basis would be reverting a correct repair.

---

## Statistics discipline — S1..S8, as properties of the API

A game-facing profiler will hit every one of these constantly, so they are **properties of the API,
not advice in a paragraph**.

| # | Rule | Enforced by |
|---|---|---|
| **S1** | **A band is `max(floor, twin, se_floor, quantum)`; no single term is the band.** | `resolve`'s signature takes both a `Floor` and a `Twin`; neither alone constructs a verdict (D11) |
| **S2** | **A zero control whose expected value is exactly zero measures DRIFT, not RESOLUTION.** P4-6's A0/A1 were the same configuration on a serialized deterministic GPU; the twin was 0 on all ten passes and the rule silently became "is nonzero", reporting a false RESOLVED. | The `se_floor` term is **mandatory** and is computed from the propagated SE of every median a reading is built from — `SE(median) ≈ 1.2533·σ/√n` (`vg_occ_split_timing.rs:329-331`, `MEDIAN_SE_FACTOR = 1.253_314_1`, with `σ̂ ≈ (p95 − median)/Z95` and `Z95 = 1.644_853_6` at `:326`). **A twin of 0 can never shrink the band below it.** |
| **S3** | **The instrument's quantum is measured per sitting, never hard-coded**, and `timestampPeriod` is not it. | `measured_quantum_ns` in the window reducer (`vg_occ_split_timing.rs:887-910`); **the plan contains no numeric GPU quantum** |
| **S4** | **Every reduced window has an ODD sample count**, so its median is an actual sample and sits on the lattice. | `WINDOW = 121`; `debug_assert!(WINDOW % 2 == 1)`; means are excluded from the quantum GCD |
| **S5** | **Never compose reduced statistics.** `median(a) + median(b) ≠ median(a+b)` — crossed by 144-240 ns in P4-6. | **No window reducer API adds two reduced values.** Partition sums are formed per frame in the frame-major row, then reduced (D7) |
| **S6** | **Two adjacent stamps cannot establish an order**, and **equal timestamps cannot license a record-order conclusion**. | `__gpu_null` deleted (D5); record order is witnessed host-side by `CommandWitness::first_pair_of` (D17) |
| **S7** | **A number whose own resolution is unknown is not printed.** | Quantum `UNKNOWN` ⇒ every GPU number in that report is `NOT RESOLVED` (D11a) |
| **S8** | **An incomplete window produces no verdict.** | `NotResolvedReason::{WindowIncomplete, EpochBreak, LabelNotMeasured}` (D11) |

---

## D7 — The stage table becomes a per-zone declaration, and partition sums are CHECKED — per frame, never over medians

`ZoneDesc.stage: GpuStage` and `ZoneDesc.group: PartitionGroup`. The window reducer **refuses to sum
a group** unless **every** member declares `BottomOfPipe` and their intervals are non-overlapping and
contained in the group's run bracket; otherwise it emits the members individually and writes
`sum = NOT_VALID (mixed stage)` — an **artifact field**, not a printed line (S1/S7 of the seam
record: the reducer has no console form).

**Why.** `begin_stage`'s argument
(`crates/boyko_rhi_vulkan/src/present/gpu_timing.rs:333-365`) is correct and currently enforced by
nobody. Verbatim from the tree: *"A `BOTTOM_OF_PIPE` stamp writes when every previously-submitted
command has COMPLETED, i.e. it is a prefix-completion time `t_k`. Prefixes are nested, so consecutive
`BOTTOM` stamps are non-decreasing and the intervals between them exactly partition their span: no
time is double-counted and none is lost. A `TOP_OF_PIPE` stamp waits only for prior commands to REACH
the top of the pipe, so it measures a different quantity — a TOP stamp recorded AFTER a BOTTOM stamp
may legally report an EARLIER time, which is why only BOTTOM-vs-BOTTOM comparisons carry the
partition property and why mixing the two is a reported observation rather than an assertion."*
Today `froxel_total_ns` sums three independent brackets and discloses it only in a prose `NOTE:`.

**New in rev 3, from P4-6 fact 3:** the sum is formed **per frame, then reduced** —
`median_f(Σ_members)`, never `Σ_members(median_f)`. `median(a) + median(b) ≠ median(a + b)`, and in
P4-6 that inequality was crossed by 144-240 ns on a real reading. The window reducer has **no** API
that adds two reduced statistics; the addition happens in the frame-major row, which is the layout
that makes it a single sequential pass (D8, owned by `profiling/store-and-fold`).

**Trade-off.** VB-P1d slots 0/1/2 stay `TopOfPipe` and can therefore never join a partition group.
Correct — **they never could**. The tree's own reason for their stage is a compatibility decision,
not a preference: *"VB-P1d's published break-even numbers are defined against a `TOP`/`BOTTOM`
bracket … and redefining the stage would silently change what an already-published number means."*

---

## D10 — Fully in-house. No Tracy stream, no Tracy protocol, v1 or v2

1. `tracy-client` is a C++ client, a build script and a TCP server process — the largest possible
   dependency against a standing zero-third-party stance.
2. **Tracy's wire format cannot represent the one property this system exists for.** `NOT RESOLVED`,
   `LOST`, a band, a measured quantum — none is expressible as a Tracy zone. Exporting would render
   them as durations, i.e. **launder unresolvable deltas back into numbers**.
3. Tracy's genuine inventions — availability-polled collection, a rejection-sampled calibration — are
   *techniques*, and we take them (D4, D3). **The protocol is not the technique.**

**Concession.** No free viewer. The dev artifact is flat TOML with `schema_version`; the session
artifact is the binary stream (D23) plus its in-tree decoder. A v1.2 optional exporter may emit
Chrome-trace JSON containing **only `MEASURED` rows** — the dropping is the exporter's *purpose*, not
its limitation.

---

## D11 — The band is `max(floor, twin)`. A `Floor` is cross-process. A `Twin` is in-sitting. Neither is a quantum

Rev 1 substituted an empty bracket, measured within one session, at 1σ. Rev 2 removed the first
substitution and **kept the other two** in `Floor::from_aa_control(control: &LegSummary, sigma: f64)`
— a single in-sitting control with a caller-supplied sigma (F4), while asserting *"`Floor` is a type
with no cheap constructor"*. And `resolve` accepted **any** `Floor` for **any** pair of legs, so a
floor measured on the VB cull class could license a delta on the SV0 class — verbatim what
`crates/boyko_app/tests/vg_decidability_floor.rs:28-30` forbids: *"a floor established on a different
instrument bounds nothing about this one."*

**Three distinct quantities, never conflated:**

| Quantity | What it is | How measured | Where it appears |
|---|---|---|---|
| **Quantum** | the instrument's own resolution | CPU: `__cpu_null` median. GPU: `measured_quantum_ns` — the GCD of every timestamp-derived value published **this sitting** (D11a) | an artifact field beside every number; a span below its channel's quantum records `BELOW QUANTUM`, never a value |
| **Floor** | the smallest defensible *relative* delta for **this workload, this box, this protocol** | `FLOOR_SIGMA = 3.0 × CV` of the **workload under test**, across `SESSIONS = 7` separate processes, `REPEATS = 3`, all three repetition floors recorded and never averaged — the `vg_decidability_floor.rs:27-73` protocol verbatim (`DEFAULT_SESSIONS = 7` at `:59`, `DEFAULT_REPEATS = 3` at `:68`, `FLOOR_SIGMA = 3.0` at `:73`) | one term of the band |
| **Twin** (the in-sitting zero control) | ongoing DRIFT during the sitting | the interleaved `zero_control` leg, reduced by `max(\|median\|, p90\|·\|)` — `vg_occ_split_timing.rs:1034` | the other term of the band |

**The reduction from three repetition floors to one `rel` is `max`, and it is a `const`-driven step,
not a caller's choice (M11).** Rev 3 said "all three repetition floors printed and never averaged"
and then handed `resolve` a scalar `Floor.rel` **without saying which of the three it was**. That is
the whole load-bearing question: the measured spread of this protocol is **6.3 / 14.3 / 4.7 / 13.5 %**
(`docs/VG-DECIDABILITY-FLOOR.md:17-20`), a 3× difference between the candidate reductions, so `min`
or a mean rebuilds the false-win machine at a different scale **while satisfying every arithmetic
check**.

- `FLOOR_REDUCTION = Reduction::Max` is a `const` in `boyko_ecs::…::profiling::floor`;
  `from_session_file` applies it and **there is no parameter**.
- **`max` is chosen because it is the only reduction that cannot manufacture a win.** A floor is a
  claim about what this instrument *cannot* decide; the honest scalar for that claim is the worst
  repetition, not the luckiest and not their average.
- **"Never averaged" is preserved and is a different statement from "never reduced".** The session
  file carries all three values and the `Floor` carries them too (`rel_all`), plus which repetition
  supplied `rel` (`rel_source_repeat`); the artifact prints all three. What is forbidden is
  collapsing them by *averaging*, which invents a value no repetition measured — exactly the defect
  that made `median(off)+median(dur)` cross a true inequality.
- **G3a gets a RED that changes ONLY the reduction:** a pinned three-floor fixture whose `min` is
  below and whose `max` is above an injected delta; with `Reduction::Max` the contrast is
  `NotResolved { BelowBand }`, with `Reduction::Min` it becomes `Resolved`. **No other input moves.**

```rust
pub struct WorkloadTag(u64);   // hash of the subscribed zone-id set + the config identity + config_tag

pub const FLOOR_SIGMA: f64 = 3.0;       // no caller-supplied sigma exists anywhere in the API
pub const FLOOR_SESSIONS: u32 = 7;
pub const FLOOR_REPEATS: u32 = 3;
pub const FLOOR_REDUCTION: Reduction = Reduction::Max;   // M11 — the honest scalar

pub struct Floor {
    rel: f64,                      // = FLOOR_REDUCTION over rel_all
    rel_all: [f64; FLOOR_REPEATS as usize],   // all three, never averaged, always published
    rel_source_repeat: u32,
    workload: WorkloadTag, sessions: u32, repeats: u32, path: PathBuf,
}
impl Floor {
    pub fn from_session_file(path: &Path) -> io::Result<Floor>;   // THE ONLY constructor
}
// deleted in rev 3: Floor::from_aa_control(control, sigma)  -- one sitting, caller-chosen sigma
// never existed:    Floor::from_quantum

pub struct Twin { ticks: u64, rounds: u32, workload: WorkloadTag }
impl Twin { pub fn from_zero_control(zero_control: &LegSummary) -> Twin; }   // no sigma parameter

pub fn resolve(a: &LegSummary, b: &LegSummary, floor: &Floor, twin: &Twin) -> Contrast;
```

**Every `Floor` in the tree is invalidated by rung 7 and re-measured at rung 7b (S1).** The floors
this project has published were measured by parsing the shipped bench's stdout
(`vg_decidability_floor.rs:133-160`, the section literally headed *"Parsing the shipped bench's own
output"*); rung 7 deletes that channel. That file's own rule (`:28-30`) applies to the migration
itself, so the artifact-channel floor is a **new** measurement with a new `WorkloadTag`, and until
rung 7b runs, every contrast returns `NotResolved { FloorWorkloadMismatch }` **through machinery that
already exists. Nothing new enforces it; the existing tag check does.**

`resolve` computes

```
band = max( floor.rel * |median_a| ,           // cross-process 3σ CV, this workload
            twin.ticks ,                       // in-sitting drift
            se_floor(a, b) ,                   // propagated SE of every median a reading is built from
            quantum_of_channel )               // sub-floor; never the whole band
```

and returns `NotResolved` — **with the delta fields still populated** — on any of:

| `NotResolvedReason` | Trigger |
|---|---|
| `BelowBand` | `\|median_delta\| <= band` |
| `FloorWorkloadMismatch` | `floor.workload != a.workload` — the check rev 2 carried the fields for and never made (F4) |
| `TwinWorkloadMismatch` | `twin.workload != a.workload` |
| `WindowIncomplete` | either leg's window carried a drop of any class (C4/X8) |
| `EpochBreak` | the legs' `clock_epoch` values differ (D3 / `substrate/clock-source`) |
| `LabelNotMeasured` | any subscribed GPU zone in either leg is `LOST` / `TORN` / `NOT_BRACKETED` |

> **Inherited naming discrepancy, flagged rather than silently resolved.** Rev 4's D11 table names
> this variant `ClockEpochBreak` while its own `enum NotResolvedReason` and its own S8 row name it
> `EpochBreak`. One word must be chosen before the rung lands. This file carries `EpochBreak`,
> because two of the three source sites use it — but the choice is *recorded as arbitrary*, not
> argued, and it is the kind of one-word divergence a reviewer of a 1957-line document does not see.

**`FLOOR_SIGMA = 3.0` is a `const`. There is no caller-supplied sigma anywhere in the API.**

**Contrast protocol: ABBA, never ABAB.** With `FRAMES_IN_FLIGHT == 2`, strict alternation aliases the
A/B phase perfectly with the frame-in-flight slot — different pool, different UBO ring slot, different
staging, forever. ABBA breaks the alias; the cancelled order bias is **reported**
(`order_bias_ticks`), not hidden. The precedent is measured, not theoretical:
`crates/boyko_app/tests/sv0_deferred_term_bench.rs:20-72` — *"The ABAB design was REFUTED by its own
null control"*: three armed sessions reported a tidy 8.3 % cross-session spread inside a 10 % gate,
and then the null control, whose true difference is exactly zero, reported a difference anyway.

**No warm-up doctrine.** Warm-up 20 → 100 was tried and **reverted as a measured negative**
(`crates/boyko_app/src/runner.rs:158-172`): raising it 5× moved the offending half-ratios from
`0.55 / 0.53` to `0.56 / 0.56` — *"no effect at all"* — while introducing a new outlier elsewhere, so
*"the ramp is **not** a settling transient"* but ongoing drift, and *"a longer warm-up simply samples
a different part of the same drift."* Instead every window records `median_first_half` /
`median_second_half` as artifact fields, so **drift is visible rather than assumed away**.

### D11a — The GPU quantum is measured per sitting and never written down

With `__gpu_null` deleted (D5/F6), the GPU quantum comes from the tree's own estimator, generalised
into the window reducer: **the GCD of every timestamp-derived value the sitting published**
(`vg_occ_split_timing.rs:887-910`). Three properties are carried verbatim because each was earned:

1. **`VkPhysicalDeviceLimits::timestampPeriod` is NOT this number.** It is the tick→ns *scale*
   (1.0 ns on this vendor), not the counter *increment* (`:893-895`). Flooring a band at
   `period × 1 tick` would satisfy every arithmetic check while silencing the alarm.
2. **Means are excluded from the GCD** (`:903-904`): *"an arithmetic mean of `n` lattice values is
   not itself on the lattice. The medians are only on it because `DEFAULT_BENCH_FRAMES` is odd."*
   Only odd-`n` medians enter.
3. **The number is computed, not hard-coded.** The odd-budget sitting measured **32 ns**; an earlier
   even-budget sitting measured 16, and the tree now records *why* both readings were internally
   honest (`:896`). Whichever is right today, a constant in this plan would be wrong tomorrow. The
   window reducer writes **the quantum it measured and the count of values it was derived from** into
   the artifact; if the sitting published no nonzero value, the quantum is `UNKNOWN` and **every GPU
   number in that artifact is `NOT RESOLVED`**.

---

## D13 — Counters and gauges are typed at the WINDOW level, so the wrong statistic is unrepresentable

`ZoneKind ∈ { Span, Counter, Gauge }`, and the accessors are kind-specific:

```rust
fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;   // None on wrong kind
fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
```

`rate_per_frame` exists only on `CounterWindow`; `median_frame_ticks` only on `SpanWindow`. Rev 1 put
all three on one `ZoneWindow` and **panicked** on the wrong kind — a runtime panic in a library API
against the repo's `Option` / `expect("invariant: ...")` convention.

flecs types this (`ecs_gauge_t` vs `ecs_counter_t`); Unreal types it; Bevy does not, which is exactly
why *"an average frame count would be nonsensical"* is special-cased in a plugin.

**Counter authoring rules — `VbRecordProbe`'s three, promoted to contract**
(`crates/boyko_rhi_vulkan/src/present/passes/vb.rs:86-100`; the struct and its per-field contract at
`:107-156`; the increments themselves at the recorder's `vkCmd*` sites — `:1710`, `:1948`, `:2313`,
`:2583`, `:2798`):

1. **Counts originate AT the operation they count** — at the `vkCmd*` call, inside the cull loop —
   never re-derived on the host. Verbatim: *"A host that re-derives `scopes` from
   `GBufferScene::vb_occlusion_instances` agrees with itself no matter what this function did — the
   tautology this campaign has shipped as a gate five times."*
2. **Host memory, not a device buffer.** *"A buffer would add an allocation, a declared pass, a
   barrier, a fence wait and a decode to move a number that is already in a register — and would
   change the recorded command stream."*
3. **What a counter cannot claim is a field of the artifact**, not a prose paragraph. The tree's own
   probe stops at *"the host recorded it"* and says so in its doc.

**Allocation counting.** The 19 zero-alloc gates each install a **process-global** allocator, which
is why they can only be test binaries. **The profiler installs no global allocator.** An opt-in
`profiling-alloc` feature in `boyko_app` installs a counting shim feeding the `Counter` channel; off
by default, `#[cfg]`-excluded at retail tier, and **its perturbation is stated in the artifact when
on**.

---

## The instrument is outside its own number — the statistics half of D16

**D16 itself is owned by `profiling/store-and-fold`** (it is a fold-placement decision). Two of its
consequences are statistics rules and are stated here so a reader of this file is not left to infer
them:

- **`instrument_estimated` is never subtracted from anything.** Rev 2 defined
  `instrument = Σ __fold + __reduce + __cpu_null + zone_count × measured_zone_cost` and then printed
  `run_net = run_gross − instrument`. The last term is **an estimate from a different binary and a
  different profile, injected into a per-frame number** — in the document that refuses to print
  unresolvable deltas and that cites `median(off)+median(dur) ≠ median(off+dur)`. It is **S5 by
  another route**: composing a measured quantity with a reduced one from a foreign sitting. So
  `instrument_measured` (the instrument's own zones, measured in-band this frame) is the only
  subtrahend; `instrument_estimated` is recorded **beside**, labelled, carrying
  `zone_cost_provenance` (bench id + `build_hash`).
- **The primary CPU number is `__frame`, and it is one interval by construction.** Rev 2's *"the
  `Schedule::run` span"* is not one interval: `crates/boyko_app/src/runner.rs:943` documents the
  frame as *"`update_with_delta` — Time → events → Fixed×N → Main"* — **two schedules, and `Fixed`
  runs N times**. A statistic reduced over a quantity whose cardinality varies per frame is not a
  statistic; `FrameRecord.fixed_steps` records N so the cardinality is in the data.

---

## Data structures — the statistics and contrast block

```rust
// ══════════════ boyko_ecs::ecs::core::profiling ══════════════

pub const WINDOW: usize = 121;                  // ODD, deliberately (S4). ~2.02 s at 60 Hz
const _: () = assert!(WINDOW % 2 == 1);
pub const MAX_LEGS: usize = 8;
pub const CONTRAST_ZONES: usize = 16;

/// Hash of the subscribed zone-id set + the config identity + `config_tag` (S10).
/// The tag is what makes "a floor measured on one workload cannot license a delta on
/// another" a CHECK rather than a convention (F4).
pub struct WorkloadTag(u64);

pub const FLOOR_SIGMA: f64 = 3.0;
pub const FLOOR_SESSIONS: u32 = 7;
pub const FLOOR_REPEATS: u32 = 3;
pub const FLOOR_REDUCTION: Reduction = Reduction::Max;

pub struct Floor {
    rel: f64, rel_all: [f64; FLOOR_REPEATS as usize], rel_source_repeat: u32,
    workload: WorkloadTag, sessions: u32, repeats: u32, path: PathBuf,
}
pub struct Twin { ticks: u64, rounds: u32, workload: WorkloadTag }

/// 48 B, in the `legs` arena. `resolve` consumes SUMMARIES, never live windows —
/// rev 1's `ZoneWindow` borrowed the live ring, so leg A's data was overwritten
/// before leg B ended (A5).
pub struct LegSummary {
    zone: ZoneId, median: u64, p95: u64, n: u32,
    labels: LabelCensus,
    first_half: u64, second_half: u64,
    drops_in_window: u32,
    clock_epoch: u16,
    workload: WorkloadTag,
}

pub enum NotResolvedReason { BelowBand, FloorWorkloadMismatch, TwinWorkloadMismatch,
                             WindowIncomplete, EpochBreak, LabelNotMeasured }

pub enum Contrast {
    Resolved    { median_delta_ticks: i64, p10: i64, p90: i64, n: u32, band_ticks: u64,
                  floor_ticks: u64, twin_ticks: u64, se_floor_ticks: u64, quantum: Quantum,
                  order_bias_ticks: i64, control_cv: f32 },
    NotResolved { reason: NotResolvedReason, /* …the same fields, all populated… */ },
}

pub struct ContrastPlan { /* ABBA sequence + leg boundaries */ }

pub enum Quantum { Known(u64), Unknown }        // S7: `Unknown` ⇒ NOT RESOLVED, never a value
```

**Why `Contrast` populates the delta fields on the `NotResolved` arm.** A reader who is told only
*"not resolved"* re-runs the measurement; a reader who is told *"not resolved, delta 180 ns, band
420 ns, of which twin 400"* knows which term to attack. Refusing to *print* an unresolvable delta and
refusing to *carry* it are different decisions, and only the first is taken.

**Why there is no third return.** `resolve` returns `Resolved{..}` or `NotResolved{reason,..}` —
**there is no bare-delta constructor anywhere in the API**, which is the structural refusal of the
seventh question in `profiling/goal-and-audiences`.

---

## Public API — the statistics and contrast slice

```rust
// ── reading — kind-specific, so the wrong statistic is unreachable (D13) ──
impl Profiler {
    pub fn span(&self, id: ZoneId)    -> Option<SpanWindow<'_>>;
    pub fn counter(&self, id: ZoneId) -> Option<CounterWindow<'_>>;
    pub fn gauge(&self, id: ZoneId)   -> Option<GaugeWindow<'_>>;
    pub fn quantum(&self, ch: Channel)-> Quantum;                  // Known(u64) | Unknown (S7)
    pub fn clock(&self)               -> ClockCalibration;
    pub fn clock_epoch(&self)         -> u32;                      // boyko_diag::clock's (S4)
}

pub struct SpanWindow<'a> { /* borrows the frame-major columns */ }
impl<'a> SpanWindow<'a> {
    pub fn median_frame_ticks(&self) -> Option<u64>;   // over per-frame TOTALS; n is ODD (S4)
    pub fn p95_frame_ticks(&self)    -> Option<(u64, u64, u64)>; // (p95, lo, hi) order-stat span
    pub fn mean_frame_ticks(&self)   -> Option<f64>;   // O(1), cached sum; EXCLUDED from S3's GCD
    pub fn per_sample_min_max(&self) -> Option<(u32, u32)>;      // distinct unit, distinct name
    pub fn halves(&self) -> (Option<u64>, Option<u64>);          // drift, always printed
    pub fn labels(&self) -> LabelCensus;
    pub fn n(&self) -> u32;
}
impl<'a> CounterWindow<'a> { pub fn rate_per_frame(&self) -> Option<f64>; pub fn level(&self) -> u64; }
impl<'a> GaugeWindow<'a>   { pub fn median(&self) -> Option<u64>; pub fn min_max(&self) -> Option<(u64,u64)>; }

// ── contrast: the ONLY way a delta leaves this system ──
impl Floor { pub fn from_session_file(path: &Path) -> io::Result<Floor>; }   // the ONLY ctor
impl Twin  { pub fn from_zero_control(control: &LegSummary) -> Twin; }        // no sigma param
pub fn resolve(a: &LegSummary, b: &LegSummary, floor: &Floor, twin: &Twin) -> Contrast;

impl ContrastPlan {
    pub fn abba(rounds: u32, frames_per_leg: u32, zones: &[ZoneId]) -> Self;
    pub fn next_leg(&mut self) -> Option<Leg>;      // the CALLER applies the A/B configuration
    pub fn seal_leg(&mut self, p: &mut Profiler);   // folds the live window into a LegSummary
    pub fn summaries(&self) -> &[LegSummary];
}

// ── artifact ──
pub fn append_artifact(p: &Profiler, path: &Path) -> io::Result<()>;   // #[cold], TOML, dev only
```

From the corpus-wide **Deliberately absent** list (carried in full by `profiling/emission-abi`), the
clauses this file is responsible for making true: **any function returning a bare delta** · **any
`ns` value without its `calib_cv`** · **any accessor that panics on the wrong `ZoneKind`** · **any
`Floor` constructor taking a sigma or a single sitting** · **any point-estimate quantile from a
histogram**.

**Vocabulary held by `seam/vocabulary`, restated here only as constraints on these names:** `window`
is reserved for **this** file's statistics horizon — the OS object is `os_window` and frame time is
`presented` frame time; the profiler says **`budget`**, never `target`, because `LogTarget` is the
logging plan's sink type; and `retention_tier` is never written bare as `tier`, because `ZoneTier` is
the other one.

---

## A4 — `WindowReducer`: window reduction, median, overlap (`#[cold]`) — and it does NOT print

**The reducer emits FIELDS, never lines.** Rev 3's reducer printed; rev 4 gives it **no console form
at all**. Every value it produces goes into the TOML artifact or the binary stream; the
human-readable rendering is `tools/prof_decode` offline and the `boyko_ui` overlay in-process (D25).
This is not a style change — **it is what lets rung 7 delete the stdout measurement channel**, and it
is why `vg_decidability_floor.rs` and its five siblings must be migrated in the same commit.

- **Reduction:** strided gather per column, `WINDOW = 121` reads per zone per column; AVX2 8-wide
  over the gathered scratch.
- **Median/p95:** copy 121 values into stack scratch, sort, index. **`n` is odd, so the median is an
  actual sample and sits on the lattice** (S4). p95 at `n = 121` is the 115th order statistic —
  recorded with its neighbours (`p95_lo`, `p95_hi`) **so its rank uncertainty is in the artifact
  rather than implied**.
- **Cost, and who pays it (M7):** one gather + one sort per zone per statistic. This is the dominant
  term of a telemetry window and is budgeted and benched as `__telemetry_reduce`, capped at
  `MAX_TELEMETRY_QUANTILE_ZONES = 64` (D23). The artifact path is `#[cold]` and **off**-frame; the
  telemetry path is `#[cold]` and **in**-frame, which is why only the latter carries a cap.
- **Partition sums:** formed **per frame in the row**, then reduced (S5). **There is no API that adds
  two reduced values.**
- **Quantum:** `measured_quantum_ns` over every timestamp-derived value the sitting published,
  **means excluded** (S3/D11a).
- **Overlap (analysis only):** per compatible pair that both ran, interval intersection over the
  `intervals` ring — SoA `u64` compare, 4-wide. O(pairs that actually ran × `OVERLAP_FRAMES`), not
  O(S²) per frame.

## A5 — Leg sealing (contrast)

`ContrastPlan::seal_leg` folds the current window's ≤ 16 subscribed zones into a `LegSummary`
(48 B) in the `legs` arena. **`resolve` consumes summaries, never live windows** — rev 1's
`ZoneWindow` borrowed the live ring, so leg A's data was overwritten before leg B ended.

**Four fields are load-bearing, not decoration:** `drops_in_window != 0` ⇒ `WindowIncomplete`; a
differing `clock_epoch` ⇒ `EpochBreak`; a `workload` mismatch against the `Floor` or `Twin` ⇒ the
corresponding mismatch reason; a non-`MEASURED` label ⇒ `LabelNotMeasured` (D11/F4/X8).

The A/B *configuration change* is applied by the caller (it must be — it **is** configuration); the
plan owns only the sequence and the boundary signal.

## A6 — Floor session (offline, N processes)

`vg_decidability_floor.rs`'s protocol generalised: the **same** workload class in
`FLOOR_SESSIONS = 7` separate processes, `FLOOR_REPEATS = 3` times, `FLOOR_SIGMA = 3.0 × CV` of the
worst subscribed statistic, **all three repetition floors written out, never averaged**, into
`docs/PROFILING-FLOOR.md` together with the `WorkloadTag` they were measured on.
`Floor::from_session_file` reads the file, carries all three in `rel_all`, and reduces them to `rel`
by `FLOOR_REDUCTION = Reduction::Max` — a `const` step, **not a caller's choice** (M11). `resolve`
checks the tag (F4).

**The sessions read the ARTIFACT, not stdout.** `vg_decidability_floor.rs` today parses the shipped
bench's printed `VB-P1d …` line (`:133-160`); after rung 7 there is no such line, so A6's per-session
input is the profiler's own artifact file. That is a **different instrument**, so it is a different
floor — rung 7b — and until it runs every `Floor` in the tree is stale **by that file's own rule**
(`:28-30`).

> **Why the protocol's own numbers must be read carefully.** `DEFAULT_REPEATS = 3` exists because
> *"the first draft measured the floor twice and got 6.3 % and 14.3 % — the same protocol, the same
> scene, the same box, a factor of 2.3 apart"* (`vg_decidability_floor.rs:63-64`). **The floor
> ESTIMATOR is itself noisy.** That is the entire argument for `Reduction::Max`, and it is why a
> single run's number quoted as "the floor" is the same over-confidence one level up.

---

## Where this file's machinery is consumed by a JOINT gate

`SEAM.md`'s **`GJ1`** — the measured off-cost of the free-when-off requirement — takes its verdict
from **`resolve`**: the same `band = max(floor, twin, se_floor, quantum)`, the same
`NotResolved{reason}` discipline, the same `WorkloadTag` check. Two properties of this API are what
make that gate non-vacuous and are recorded here because they are *this file's* obligations:

- **A two-leg A/B cannot distinguish "the flag is off" from "the sites were never compiled in."**
  GJ1's third leg exists for that reason, and if it does not resolve apart from the flag-off leg the
  gate reports `NOT RESOLVED (control inert)`. **That is the `__gpu_null` lesson generalised** — a
  control that is measured-inert is not a control, and a band built on it collapses to its other
  terms.
- **A verdict taken against a baseline whose `config_tag` differs is refused, not failed.**
  `WorkloadTag` folds `config_tag` in, so a sitting compared against a foreign-configuration baseline
  returns `NotResolved` and records `UNPROVEN`. The rule that no regression gate may *fail* a rung
  before the joint baseline sitting **J2** is `seam/landing-order`'s, and this API is the mechanism
  that makes it automatic rather than remembered.

`resolve`'s inputs are windows and summaries only, and neither is touched while the runtime flag is
off — the reducer is `#[cold]`, off the emission path, and runs only over a sealed window that a
disarmed profiler never produces. **No statistics state is calibrated, allocated or touched at boot.**

---

## Citations re-verified at the carve (2026-08-08, against HEAD)

Confirmed unchanged: `vg_decidability_floor.rs:28-30` (*"a floor established on a different
instrument bounds nothing about this one"*), `:59` (`DEFAULT_SESSIONS: usize = 7`), `:68`
(`DEFAULT_REPEATS: usize = 3`), `:73` (`FLOOR_SIGMA: f64 = 3.0`), `:63-64` (the 6.3 %/14.3 %
irreproducibility), `:133-160` (*"Parsing the shipped bench's own output"*);
`docs/VG-DECIDABILITY-FLOOR.md:17-20` (the four floor readings);
`crates/boyko_app/src/runner.rs:158-172` (warm-up 20→100 tried and reverted) and `:943`
(*"Time → events → Fixed×N → Main"*); `sv0_deferred_term_bench.rs:20-72` (ABAB refuted by its null
control); `gpu_timing.rs:333-365` (`begin_stage` and the prefix-completion argument);
`vb.rs:86-100` (the originate-here and no-device-buffer arguments);
`schedule_builder.rs:70` (`MAX_SYSTEMS_PER_SCHEDULE = 1024`).

**Corrected while carrying.** Rev 4's citations into `crates/boyko_app/tests/vg_occ_split_timing.rs`
are **stale by roughly one file revision**; every one was re-located. The arguments are unaffected —
only the line numbers moved — but a reader following rev 4's numbers lands on unrelated code, and in
one case on a blank line:

| Rev 4 cited | Actually at | What is there |
|---|---|---|
| `:834` — the twin term | **`:1034`** | `fn twin_term(&self) -> f64 { band_of(&self.zero) }`. Rev 4's `:834` is a **blank line** |
| `:867` — the floor | `:867-871` ✓ | `fn resolution_of(...)`; its sum `floor_over` is at `:883-885` |
| `:871-892` — `measured_quantum_ns` | **`:887-910`** | the doc block `:887-909`, the fn at `:910` |
| `:879-881` — "`timestampPeriod` is NOT this number" | **`:893-895`** | the ⚠️ paragraph |
| `:885-886` — means excluded | **`:903-904`** | *"an arithmetic mean of `n` lattice values is not itself on the lattice"* |
| `:315` — `SE(median) ≈ 1.2533·σ/√n` | **`:329-331`** | `MEDIAN_SE_FACTOR = 1.253_314_1`; `Z95 = 1.644_853_6` at `:326`. Rev 4's `:315` is the *odd-budget* note |
| `:301-306` — `DEFAULT_BENCH_FRAMES = 221` is odd | **`:315-322`** | rev 4's `:301-306` is `DEFAULT_ROUNDS` / `DEFAULT_LONG_FRAMES` |
| `vb.rs:107-156` — "increment sites" | `:107-156` is the **struct**; increments at `:1710`, `:1948`, `:2313`, `:2583`, `:2798` | the rule is unaffected |

**One claim found FALSE against the tree and re-cut above:** rev 4's *"two prose sites in that same
file still say 16 ns (`:138`, `:881`)"*. Both `16 ns` occurrences today (`:141`, `:896`) are
retractions that explain the even-budget cause, and `:138` states the *opposite* of the attributed
text. See "Fact 6's in-tree doc-rot is REPAIRED IN THE TREE" above.

**One imprecision worth not propagating:** rev 4 describes 6.3 / 14.3 / 4.7 / 13.5 % as *"four runs
of one protocol"*. `docs/VG-DECIDABILITY-FLOOR.md:17-20` records them as **two runs of a
peak-to-peak statistic and two of a CV-derived one**, the statistic having been changed after run 2
refuted peak-to-peak. The load-bearing point survives intact and is arguably stronger: **both**
statistics reproduced themselves only to within 2.3× and 2.9×.
