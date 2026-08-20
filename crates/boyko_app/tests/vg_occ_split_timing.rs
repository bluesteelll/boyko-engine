//! **VG R3 piece 4 rung P4-6 — THE MEASUREMENT: what the occlusion split costs, per pass, with
//! every band taken from a zero control interleaved inside the same round.**
//!
//! Piece 3's step P3-8 shipped this file with two channels and one honest conclusion: *this
//! repository has no instrument that can see the occlusion split's GPU cost.* Rungs P4-1 and P4-2
//! built one — a totality epilogue that makes an unbracketed pass observable instead of a hang, and
//! ten timestamp brackets over the split's own passes. This rung uses it.
//!
//! **No performance threshold is asserted and no benchmark is pinned** (step P3-8's rule, kept).
//! The numbers are published as prose in the P4-6 commit message. What this file asserts is
//! instrument-level only, and it is listed in full under "What this harness ASSERTS" below.
//!
//! # ⚠️ THE INSTRUMENT FACT THIS FILE USED TO STATE, AND ITS REPAIR
//!
//! Until this rung the header of this file said, at line 30: *"`VbTimedPass` has exactly three
//! members — `CullReset`, `CullDispatch` and `VbShade`. Nothing brackets `vb_batch_cull`,
//! `vb_cull_late` or the late raster scope."* **That has been FALSE since rung P4-2**, which took
//! the enum 3 → 10 and bracketed exactly those three things. The sentence is deleted rather than
//! softened: a stale instrument fact is worse than a missing one, because a reader who trusts it
//! discards the very channel that now decides.
//!
//! # ⚠️ THE FLOOR IS NOT A CONSTANT
//!
//! `docs/VG-DECIDABILITY-FLOOR.md` records four runs of ONE protocol on this box producing floors of
//! **6.3 / 14.3 / 4.7 / 13.5 %**. The floor drifts on a timescale shorter than the gap between two
//! runs, so its operational conclusion is a rule and not a threshold —
//!
//! > a claimed GPU-timing delta below ~15 % is not defensible without a NULL CONTROL measured in the
//! > same sitting
//!
//! — and every band below is therefore computed by running the **identical reduction** on a zero
//! control that is interleaved **inside every round**, never as a separate session. Where a band
//! would be vacuous, none is claimed.
//!
//! # The two channels
//!
//! | channel | what it is | status |
//! |---|---|---|
//! | **G** | the ten VB timestamp brackets, read through the `BOYKO_VB_ZONE` recorder's artifact rows (they were the `BOYKO_VB_BENCH` collector's until rung 7) | **the deciding channel** |
//! | **W** | host wall clock across the frame loop, two-point subtracted | **`KNOWN-BLIND`** |
//!
//! **Channel W decides nothing, and that is a measured conclusion rather than a caution.** The
//! swapchain is created with `VK_PRESENT_MODE_FIFO_KHR` unconditionally
//! (`crates/boyko_rhi_vulkan/src/present/swapchain.rs`), so the host loop is bounded below by the
//! display refresh. Piece 3's sitting measured 6.893 ms/frame = 145.1 Hz with a zero control of
//! 0.47 % against a 3σ band of 287.91 %: every contrast came back `NOT RESOLVED`. W is kept for
//! exactly ONE claim — *did arming the instrument WRECK the frame?* — at a threshold of **> 20 %**
//! frame-period inflation over `A0`, about 1.4× its own worst measured floor (14.3 %). Below that
//! it reports "inside my own noise" and nothing else.
//!
//! # The protocol
//!
//! Per ROUND, four legs run back to back in one order, `A0 → B → C → A1`, and each leg's W and G
//! measurements are taken back to back **at that leg's position in the round**, so both channels'
//! controls span exactly the drift their contrasts are exposed to:
//!
//! | leg | configuration |
//! |---|---|
//! | `A0` | markers PRESENT, `OcclusionMode::Off`, `HzbConfig::Build` — the disarmed baseline |
//! | `B` | markers present, `TwoPhase` + `OcclusionForce::KeepAll` — every mechanism, no decision |
//! | `C` | markers present, `TwoPhase` + `OcclusionForce::None` — the real decision |
//! | `A1` | `Off` again, markers present — **the zero control** |
//!
//! **`A0`/`A1` are `Off` WITH markers, not marker-absent.** Until this rung the disarmed leg
//! withheld the `OcclusionCulling` component, which moves two variables at once — the arming
//! predicate *and* the spawn flush plus the `occlusion_instances` accounting. `Off`-with-markers is
//! the one-variable contrast, and it is the only leg that exercises `OcclusionConfig`'s "do not
//! TEST, never do not GATHER" semantics. **All four legs insert `HzbConfig { mode: Build }`**, so
//! the pyramid exists everywhere and `vb_hzb_build` differs only in POSITION, never in existence.
//!
//! ⚠️ **What this does not measure:** the marker's own gather cost. The difference between a
//! marker-absent boot and `Off`-with-markers is on no leg of this protocol and is not claimed.
//!
//! `A0` and `A1` are the same configuration, so the driver spawns them with **byte-identical
//! environments** — the slot is a driver-side notion and never reaches the worker. A control whose
//! process differed from its twin in even one env string would not be a control.
//!
//! # The quantities (every one is a COST in nanoseconds; positive = more time)
//!
//! Per round `r`, per pass `p`, per leg `L`, `m_p(L)` is that worker's own median over its timed
//! frames, and `Δ_p := m_p(C) − m_p(B)`.
//!
//! | symbol | definition | sign meaning |
//! |---|---|---|
//! | **`NetRun`** | **`Δ_9`** | **THE HEADLINE.** `+` ⇒ the decision costs more than it saves |
//! | `Saving` | `−Δ_5` | `+` ⇒ the decision SHRINKS the early raster. Attribution-grade |
//! | `Overhead` | `Δ_3 + Δ_4 + Δ_7 + Δ_8` | attribution only — not a bound in either direction |
//! | `Net` | `Overhead + Δ_6 − Saving` | the per-slot attribution sum over all six in-run slots |
//! | `GapResidual` | `NetRun − Net` | must be ≈ 0 — the gap commands are leg-independent |
//! | `HzbResidual` | `Δ_6` | must be ≈ 0 — same dispatch chain, same site on `B` and `C` |
//! | `Residual` | `Δ_0 + Δ_1 + Δ_2` | must be ≈ 0 — three passes carrying no split-dependent work |
//! | `PlumbRun` | `[m_9(B) − m_6(B)] − m_9(A0)` | `+` ⇒ arming the mechanism costs. Attribution-grade |
//! | `Bracketed` | `Σ_{p∈0..8, p≠6} [m_p(C) − m_p(A0)]` | `+` ⇒ the bracketed ranges cost more armed |
//! | `LateShare` | `Overhead / m_5(A0)` | the machinery's cost as a fraction of the raster it shrinks |
//!
//! **`NetRun` is `Δ_9` and nothing is subtracted from it.** `Δ_9` is a paired difference of two
//! intervals delimited by stamps at identical positions and identical stages, on command streams
//! that differ only in one push-constant bit and the indirect counts it produces. Within-run
//! migration between slots 3–8 is zero-sum by the `BOTTOM_OF_PIPE` partition property and cancels
//! exactly. `B` and `C` are both split legs, so `vb_hzb_build` sits at the same recorder site on
//! both — subtracting `Δ_6` would cure an armed-vs-disarmed property that does not exist here and
//! would only inject variance. `Δ_6` is reported separately as `HzbResidual`, with its own band.
//!
//! **`NetRun`'s residual bias is second-order and UNSIGNED.** Only the run's two outer boundaries
//! stay exposed: work overlapping the tail of pre-run commands is charged before `b9` (deflates),
//! work that would otherwise overlap post-run commands is waited for at `e9` (inflates). Both are
//! present with the same structure on `B` and `C`, so they are paired and cancel to first order.
//! What does not cancel is a count-dependent change in how much run work is available to overlap a
//! boundary. **No directional confidence is claimed for either sign of the result.**
//!
//! # ⚠️ BANDS — and the PLAN-LEVEL DEFECT rung P4-6 measured in §C2
//!
//! `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` §C2 defines the band as the zero twin ALONE: the
//! identical reduction run on `A1`-vs-`A0`, `max(|median_r Q⁰_r|, p90_r |Q⁰_r|)`. **That is a DRIFT
//! estimator and only a drift estimator**, and `A0`/`A1` are the SAME configuration — same scene,
//! byte-identical environment, same command stream — on a fully serialized frame. On a
//! deterministic GPU its expected value is therefore **exactly zero**.
//!
//! P4-6's third sitting measured exactly that: `band(NetRun) = 0` across five rounds of ten
//! separate processes, with `m_9` a perfectly healthy 47 104 ns (`vb_occ_mixed`) / 691 200 ns
//! (`vb_occ_dense`), while channel W's own legs wandered 6–9 % in the same sitting. With drift at
//! zero the verdict rule collapses from *"|Q| clears the instrument's noise"* to **"Q ≠ 0"** — the
//! false-win machine — and `Saving` duly read `RESOLVED` on eight low-poly instances at 512².
//!
//! **The zero control is not at fault and is not removed.** It did its job and reported honestly:
//! there is no process-to-process drift on the GPU channel. What it structurally cannot supply is
//! **RESOLUTION**, the other half of what a verdict needs. §C2 conflated the two.
//!
//! So `band(Q) = max(FLOOR, TWIN)`, both printed beside every quantity:
//!
//! * **FLOOR** — the reading's own resolution: the propagated standard error of every median it is
//!   built from, `SE ≈ 1.2533·σ̂/√n` with `σ̂ ≈ (p95 − median)/1.645`, taken from the `p95_ns`/`n`
//!   the runner already publishes, on the SAME legs the reading uses, in THIS sitting. Sub-floored
//!   per median at the measured timestamp lattice, because a pass reporting `p95 == median` would
//!   otherwise claim resolution finer than the counter can represent.
//! * **TWIN** — §C2's drift term, unchanged.
//!
//! ⚠️ The lattice is **measured** ([`measured_quantum_ns`]) and is **not**
//! `VkPhysicalDeviceLimits::timestampPeriod`, which is the tick→ns SCALE (`1.0` here) and says
//! nothing about how often the counter increments. It came back **32 ns** in the sitting this text
//! describes. An earlier draft of this paragraph said 16 ns: that GCD was taken over medians
//! produced under an EVEN frame budget, where each median is the mean of the two middle samples and
//! can land a half-tick off the lattice — which is why [`DEFAULT_BENCH_FRAMES`] is odd. **Neither
//! number is hard-coded and neither is a constant of the machine**; the harness re-measures the
//! quantum from the values each sitting publishes. Flooring at `period × 1 tick` instead would have
//! satisfied every assertion in this file while under-stating the resolution by the whole lattice
//! factor (32× as measured here): the alarm silenced, the false win intact.
//!
//! **Rung P4-7 LANDED the plan's repair.** `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` §C2 now
//! defines `band(Q) = max(FLOOR, TWIN)` and records this finding as the reason, so the plan and this
//! harness no longer disagree about what a band is.
//!
//! A sum's band is that sum's own band; no per-pass band is applied to an aggregate.
//!
//! **Where a band would be vacuous, none is claimed.** `A0` and `A1` are both disarmed, so slots 3,
//! 7 and 8 bracket empty blocks on both and their zero-twin band is the lattice quantisation:
//! testing "the late passes cost more than nothing" against it is unfalsifiable *and* trivially
//! true. Those are reported as magnitudes with a scale (`LateShare`), never as significance
//! verdicts. Significance bands are claimed for exactly six quantities whose **sign is the claim**:
//! `NetRun`, `Saving`, `GapResidual`, `HzbResidual`, `Residual`, `Bracketed`.
//!
//! # ⚠️ `Bracketed` IS NOT END-TO-END, and this repository still has no end-to-end number
//!
//! Outside the brackets: everything before `CullReset`, the CSM cascade loop, the punctual atlas
//! loop, the sky scope, the classify chain, `vb_viewt`/`vb_geo`/SSAO/à-trous under the split
//! producer, `sdf_forward_march`, the present blit, and the whole non-`record_vb` frame. Channel W,
//! the only end-to-end channel, is `KNOWN-BLIND`.
//!
//! # Scope limits of EVERY number this file prints
//!
//! * Measured on a **fully serialized** frame: `ctx.wait_idle()` on every bench frame, on top of
//!   unconditional FIFO. Correct for timestamp deltas; **not** the frame the shipped renderer
//!   executes. A small `Bracketed` does not imply a short critical path.
//! * One machine, one driver, one sitting, two fixtures, static scenes.
//! * `Saving` **under motion** is not measured and cannot be: the pyramid is a fixed point on a
//!   static scene (plan D12). The early phase's hit rate under motion is piece 3's OQ 3, still open.
//! * The marker's own gather cost is on no leg.
//! * The `BOYKO_VB_CULL_READBACK` probe's cost is on no leg — the runner refuses the probe and the
//!   bench together at boot, so no published timestamp contains any part of it.
//! * The dev-profile dual-read equality invariant does not execute in a release bench run.
//!
//! # What this harness ASSERTS
//!
//! Instrument-level only, and — per rung P4-2's stage rule — **`BOTTOM`-vs-`BOTTOM` comparisons
//! only**. Slots 0–2 keep `TOP_OF_PIPE` begins for VB-P1d compatibility, and a `TOP` stamp waits
//! only for prior commands to *reach* the pipe top, so a later-recorded `TOP` may legally report an
//! earlier time. Every clause that would compare slot 2's begin with a `BOTTOM` stamp is printed as
//! an `OBSERVATION` and decides nothing.
//!
//! 1. every leg produced `rounds` samples on every pass, and no pass was flagged `FALLBACK` or
//!    `TORN` on any leg;
//! 2. every worker's `VB-P4 regime` line reports `n_distinct == 1` and the regime expected for its
//!    leg;
//! 3. `begin_offset_ns` is monotone across the leg-independent run `b9, b3, b4, b5, b7, b8`, plus
//!    `off(8) + dur8 ≤ off(9) + dur9`, on all four legs. **Every pair in that chain is asserted
//!    `≤`**, per the adjacent-stamp audit in [`BEGIN_CHAIN`];
//! 4. slot 6's placement per leg — armed ⇒ `off(5) + dur5 ≤ off(6) ≤ off(7)`; disarmed ⇒
//!    `off(6) > off(9) + dur9`, the one relation in the set where strictness IS available (a whole
//!    shade dispatch separates the stamps). This half carries the "slot 6 left the run" claim;
//! 5. every `begin_offset_ns < 1e9` — the base-stamp contract (a broken one shows as ~2^36 ticks,
//!    not as a plausible number);
//! 6. every published quantity's **band** is nonzero. ⚠️ Not "the zero control is not exactly
//!    zero", which is what this list said before rung P4-7 and which the shipped clause has never
//!    been: a zero TWIN is EXPECTED here (`A0` and `A1` are one configuration on a serialized
//!    deterministic GPU) and asserting against it would red a healthy run. A zero BAND means the
//!    resolution FLOOR also came out zero — every worker reported `p95 == median` and the lattice
//!    measured `0` — i.e. the sitting published no scale at all;
//! 7. `m_8(A0) < m_8(B)` and `m_7(A0) < m_7(B)` — a zero-width bracket must read smaller than one
//!    containing real work. If this fails, **every number in the report is noise**;
//! 8. `|Residual| ≤ band`, `|GapResidual| ≤ band`, `|HzbResidual| ≤ band`;
//! 9. ~~the `VB-P1d …` line's shade mean and the `VB-P4 pass=vb_shade` line's `mean_ns` are the
//!    same number~~ — **STRUCK: profiling rung 7 deleted both printers.** The clause compared two
//!    channels reducing one sample row; every figure this file reads now comes from the artifact,
//!    so the comparison has one operand and is not a weaker clause but no clause. Struck rather
//!    than renumbered — the numbers are cited from the assertion bodies below, and a renumber
//!    would silently repoint them. Its `mean_ns` column and the `key_f64` parser went with it,
//!    each having had exactly one reader.
//!
//! # ⚠️ TWO REASONS A STAMP COMPARISON CAN HAVE NO MARGIN — they are different, and both are here
//!
//! **(a) The counter's lattice quantum.** Two `BOTTOM_OF_PIPE` stamps with NO GPU command between
//! them wait on prefixes differing by nothing, so their readings are only guaranteed to differ by
//! the timestamp counter's own quantum ([`measured_quantum_ns`] — 32 ns in the sitting this text
//! describes, re-measured every sitting). Such pairs came back EQUAL here, i.e. the counter did not
//! tick between them; that difference is `0` on this machine and legally non-zero elsewhere
//! (`vb_bench_totality_gate.rs:90-101` states the same property, as an observed value under a bound
//! rather than a pinned literal, for its zero-pair). Every such
//! pair is asserted `≤`, and `≤` is the TRUE relation, not a relaxed one. Rung P4-6's first sitting
//! asserted `off(b9) < off(b3)` strictly and both fixtures reported them EQUAL, deterministically:
//! `vb.rs:1598-1599` stamps the two on adjacent lines of one block. [`BEGIN_CHAIN`] carries the
//! per-pair audit — *what does the recorder put between these two stamps?* — so the answer is
//! derived from `vb.rs` per pair rather than assumed for the set.
//!
//! **(b) The median composition — REMOVED, not tolerated.** The two clauses that compare stamp
//! ENDS (`e8 ≤ e9`, and armed `e5 ≤ b6`) used to compute an end as `begin_off_ns + median_ns`. That
//! is `median(off) + median(dur)`, which equals `median(off + dur)` only when the begin offset is
//! constant across frames — and it is not, because the pre-run work a `BOTTOM_OF_PIPE` stamp waits
//! on jitters. P4-6's first sitting measured it: `e8` read 144 ns PAST `e9` on a 47 µs run
//! (`vb_occ_mixed`) and 240 ns past on a 691 µs run (`vb_occ_dense`), against a per-frame relation
//! that holds always. **The instrument was fixed instead of the clause**: `boyko_app::runner` now
//! publishes an eleventh key, `end_off_ns`, the median of PER-FRAME `(end − base)` values reduced
//! whole, and both clauses read it directly at full strength. No tolerance was added and no `≈`
//! appears anywhere. Reason (a) is now the only reason a comparison here can lack a margin.
//!
//! ⚠️ **Nothing in this file reconstructs an end time by adding two published medians.** The one
//! helper that did (`BenchSummary::end_of`) is deleted rather than left available.
//!
//! **⚠️ Neither (a) nor (b) says anything about the host's RECORD ORDER.** Equal timestamps mean the
//! counter did not tick. Record order is a host property; the witness for it is a host-side flag set
//! at the write and read at the dependent site — rung P4-3's `cull_uniform_filled`
//! (`vb.rs:1613`/`:1621`) — and no clause in this file infers it from a timestamp value.
//!
//! # Run
//!
//! ```text
//! $env:BOYKO_DISABLE_VALIDATION=1
//! cargo test -p boyko-app --test vg_occ_split_timing -- --ignored --nocapture --test-threads=1
//! ```
//!
//! | knob | default | what it does |
//! |---|---|---|
//! | `BOYKO_VG_OCC_TIMING_ROUNDS` | 5 | rounds of the whole four-leg sequence |
//! | `BOYKO_VG_OCC_TIMING_FRAMES` | 300 | channel W's long budget `N₂` (`N₁` is fixed at 60) |
//! | `BOYKO_VG_OCC_TIMING_BENCH_FRAMES` | **221** | channel G's TIMED frames per worker, past warm-up — **ODD deliberately**, see [`DEFAULT_BENCH_FRAMES`] |
//! | `BOYKO_VG_OCC_DENSE_K` | 64 | `vb_occ_dense`'s replication factor |
//!
//! One round costs 12 worker processes (2 per leg for W's two-point subtraction, 1 per leg for G),
//! so the defaults are 60 processes per fixture.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use boyko_app::profiling::artifact::{Artifact, ZoneLabel};
use std::process::Command;
use std::time::Instant;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::hzb::{HzbLayout, OcclusionVerdict, occlusion_verdict};
use boyko_render::{
    GeometryLegs, HzbConfig, HzbMode, Material, MeshGeometryTableSlot, OcclusionMode, RenderPath,
    RenderPathConfig,
};
use boyko_rhi_vulkan::present::{
    HZB_DUMP_FLAG_DEPTH_EARLY, HZB_DUMP_HEADER_BYTES, HZB_DUMP_HEADER_SCALAR_WORDS, HZB_DUMP_MAGIC,
    HZB_DUMP_SAMPLE_BYTES, HZB_DUMP_WORD_FLAGS, HZB_DUMP_WORD_FRAME_INDEX,
};

mod occ_fixture;
mod vb_inst_cull_scene;
mod vb_occ_dense;
mod vb_occ_mixed_scene;

use vb_inst_cull_scene::{CullProbe, parse_probe_line};
use vb_occ_dense::DenseInstance;
use vb_occ_mixed_scene::{MixedMesh, Role};

// ===============================================================================================
// Constants and knobs
// ===============================================================================================

/// The worker every leg re-executes.
const WORKER: &str = "vg_occ_split_timing_worker";

/// Rounds of the whole four-leg sequence. Five rather than three: three samples estimate a spread
/// very poorly, and the spread is what decides whether any contrast is a number.
const DEFAULT_ROUNDS: usize = 5;

/// Channel W's LONG frame budget, `N₂`.
const DEFAULT_LONG_FRAMES: u32 = 300;

/// Channel W's SHORT frame budget, `N₁` — the two-point subtraction's other end. Large enough to be
/// past every first-frame effect (pipeline creation, descriptor writes, swapchain acquisition) and
/// small enough that `N₂ − N₁` is most of the measurement.
const SHORT_FRAMES: u32 = 60;

/// Channel G's TIMED frames per worker, past the runner's own `VB_BENCH_WARMUP`. Spelled here so
/// the budget is a property of this protocol and not of whatever the shell happens to export.
///
/// ⚠️ **ODD, deliberately** — 221 rather than the runner's own default of 220. `vb_bench_stats_ns`
/// returns `0.5 × (sorted[n/2 − 1] + sorted[n/2])` for an even `n`, so every published median was
/// the MEAN OF TWO SAMPLES: a value no frame had, sitting half a tick off the timestamp lattice
/// whenever the two middle samples differ. An odd budget makes every published median an actual
/// sample. Two things depend on it — the numbers this rung publishes lose that bias, and
/// [`measured_quantum_ns`], which reads the lattice off the published values, stops being ambiguous
/// between `q` and `q/2`.
const DEFAULT_BENCH_FRAMES: u32 = 221;

/// The `z` of the 95th percentile of a normal — `p95 − median ≈ z·σ` is how [`resolution_of`]
/// recovers a dispersion from the two order statistics the runner already publishes.
const Z95: f64 = 1.644_853_6;

/// `√(π/2)` — the asymptotic ratio of the standard error of a MEDIAN to that of a mean, so
/// `SE(median) ≈ 1.2533·σ/√n`. The published statistic is a median, so the band's floor must be
/// the median's own sampling error and not the mean's.
const MEDIAN_SE_FACTOR: f64 = 1.253_314_1;

/// The env knob the worker reads to know which regime to boot in.
const ENV_LEG: &str = "BOYKO_VG_OCC_TIMING_LEG";

/// The env knob the worker reads to know which SCENE to spawn.
const ENV_FIXTURE: &str = "BOYKO_VG_OCC_TIMING_FIXTURE";

/// Channel W's ONE claim: an armed leg whose frame period exceeds `A0`'s by more than this WRECKED
/// the frame. ~1.4× W's own worst measured floor (14.3 %), so it cannot fire on noise this channel
/// has already been measured to produce.
const WRECK_THRESHOLD: f64 = 0.20;

/// The runner's diagnostic when the device cannot serve timestamps at all. A device that arms no
/// collector makes this whole rung unmeasurable — reported loudly, never as a red.
const NO_TIMESTAMPS: &str = "device timestamps are unusable";

/// The ten VB zone ids' names, in id order -- what `VbTimedPass::label()` returned before rung 7
/// deleted it. Named here rather than imported: production keys its rows by NUMERIC zone id, and a
/// name table shipped for one harness's benefit is the hand-maintained mapping D6 rejects.
///
/// ⚠️ This array is the harness's copy of `gpu_timing.rs`'s table. It is not a duplicate that can
/// drift silently: every label is looked up by name in the worker's output, so a label the recorder
/// renamed reds as "no `VB-P4 pass=<name>` line" naming the slot, instead of being averaged away.
const PASS_LABELS: [&str; PASS_COUNT] = [
    "cull_reset",
    "cull_dispatch",
    "vb_shade",
    "vb_late_upload",
    "vb_early_cull",
    "vb_early_raster",
    "vb_hzb_build",
    "vb_late_cull",
    "vb_late_raster",
    "vb_run",
];

/// The VB zone count as this harness sees it (rung P4-2 took it 3 → 10).
///
/// # It stays 10 while the family is 15, and widening it would PANIC (VB-SV0 DP6-0b)
///
/// This worker is one `VisibilityBuffer × Mesh` boot with no `SsaoConfig` / `DdgiConfig` /
/// `TaaConfig`, so on it ids **10, 11 and 13 never stamp** — no `alloc_pair`, no `PairResult`, no
/// row. They are not written-and-unread here; they do not exist. And the loop over
/// [`PASS_LABELS`] **panics** through `unwrap_or_else` on a missing row, so raising this bound to
/// cover them would be an unconditional panic on all four legs, on every run.
///
/// DP6-0b adds **TWO** rows this harness ignores, and the count is spelled because the first
/// version of this note said "exactly one":
///
/// * **id 12** (`ZONE_VB_PRODUCE_RUN`) — armed on `mesh_leg`, which this boot is, so it stamps;
/// * **id 14** (`ZONE_VB_PRODUCE_NET`) — never stamped by anything, but the reducer FORMS it every
///   frame on a fused leg, where `ZONE_VB_PRESHADE` is `Forbidden` and therefore contributes 0.0
///   (`VB_DERIVED_FUSED`). A derived row is still a row in the file.
///
/// Both are deliberate and cost nothing: the bound stops at 10, neither is ever looked up, and this
/// file's `begin_off` base is unmoved (the frame's earliest measured begin is
/// `ZONE_VB_CULL_RESET`, far ahead of the producer run).
///
/// The blindness this leaves is real and DOUBLE — blind by FIXTURE (this boot structurally cannot
/// stamp 10/11/13) and blind by BOUND — and widening the bound fixes neither. The per-leg
/// expectation table in `vb_sv0_produce_run_timing.rs` is what covers those ids, on legs that
/// stamp them.
const PASS_COUNT: usize = 10;

const P_CULL_RESET: usize = 0;
const P_CULL_DISPATCH: usize = 1;
const P_VB_SHADE: usize = 2;
const P_LATE_UPLOAD: usize = 3;
const P_EARLY_CULL: usize = 4;
const P_EARLY_RASTER: usize = 5;
const P_HZB_BUILD: usize = 6;
const P_LATE_CULL: usize = 7;
const P_LATE_RASTER: usize = 8;
const P_RUN: usize = 9;

/// The base-stamp contract: every `begin_off_ns` **and** every `end_off_ns` is relative to pair 0's
/// begin, so a frame's offsets are sub-second. A broken base shows as ~2^36 ticks, i.e. tens of
/// seconds — never as a plausible number, which is why the bound is loose rather than tuned. Both
/// offsets share one base, so they are checked by one clause.
const MAX_BEGIN_OFF_NS: f64 = 1.0e9;

// ===============================================================================================
// The legs, the slots and the fixtures
// ===============================================================================================

/// One CONFIGURATION of the four-leg round. `A0` and `A1` are two runs of [`Leg::Disarmed`], so the
/// two control processes see byte-identical environments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Leg {
    /// Markers PRESENT, `OcclusionMode::Off`. The one-variable disarmed baseline: the config's
    /// VALUE is the only thing that differs from the armed legs.
    Disarmed,
    /// `TwoPhase` with `VB_CULL_OCC_FORCE_KEEP` — every mechanism runs, the decision defers nothing.
    ForceKeep,
    /// The shipping configuration: `TwoPhase`, unforced.
    Armed,
}

impl Leg {
    const fn name(self) -> &'static str {
        match self {
            Leg::Disarmed => "disarmed",
            Leg::ForceKeep => "force_keep",
            Leg::Armed => "armed",
        }
    }

    /// `true` on EVERY leg since rung P4-6.
    ///
    /// Until this rung the disarmed leg withheld `OcclusionCulling`, which moved the arming
    /// predicate AND the spawn flush AND the `occlusion_instances` accounting together. The
    /// disarm is now `OcclusionMode::Off`, which is what the config means: do not TEST, never do
    /// not GATHER.
    const fn marked(self) -> bool {
        true
    }

    /// The `OcclusionConfig` mode this leg inserts.
    const fn mode(self) -> OcclusionMode {
        match self {
            Leg::Disarmed => OcclusionMode::Off,
            Leg::ForceKeep | Leg::Armed => OcclusionMode::TwoPhase,
        }
    }

    /// The `BOYKO_VG_OCC_FORCE` value, if any.
    const fn force(self) -> Option<&'static str> {
        match self {
            Leg::ForceKeep => Some("keep"),
            Leg::Disarmed | Leg::Armed => None,
        }
    }

    /// The regime word the worker's `VB-P4 regime observed=[…]` line must report.
    const fn expected_force_word(self) -> &'static str {
        match self {
            Leg::ForceKeep => "keep",
            Leg::Disarmed | Leg::Armed => "none",
        }
    }

    /// The mode word the worker's `VB-P4 regime … mode=[…]` line must report.
    const fn expected_mode_word(self) -> &'static str {
        match self {
            Leg::Disarmed => "off",
            Leg::ForceKeep | Leg::Armed => "two_phase",
        }
    }

    /// `true` iff `record_hzb_poison_build` is called from INSIDE the run on this leg — the whole
    /// content of clause 4's two branches.
    const fn hzb_inside_run(self) -> bool {
        matches!(self, Leg::ForceKeep | Leg::Armed)
    }

    fn parse(s: &str) -> Self {
        match s {
            "disarmed" => Leg::Disarmed,
            "force_keep" => Leg::ForceKeep,
            "armed" => Leg::Armed,
            other => panic!("`{other}` is not a timing leg"),
        }
    }
}

/// A position in the four-leg round. Driver-side only: the worker is told a [`Leg`], never a slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    /// The disarmed baseline, at the head of the round.
    A0,
    /// FORCE-KEEP: every mechanism, no decision.
    B,
    /// The real decision.
    C,
    /// The disarmed ZERO CONTROL, at the tail of the round.
    A1,
}

/// The round's order, and the only order any reduction below reads.
const SLOTS: [Slot; 4] = [Slot::A0, Slot::B, Slot::C, Slot::A1];

impl Slot {
    const fn leg(self) -> Leg {
        match self {
            Slot::A0 | Slot::A1 => Leg::Disarmed,
            Slot::B => Leg::ForceKeep,
            Slot::C => Leg::Armed,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Slot::A0 => "A0 (Off, baseline)",
            Slot::B => "B  (TwoPhase, KEEP)",
            Slot::C => "C  (TwoPhase, real)",
            Slot::A1 => "A1 (Off, zero ctrl)",
        }
    }

    const fn idx(self) -> usize {
        match self {
            Slot::A0 => 0,
            Slot::B => 1,
            Slot::C => 2,
            Slot::A1 => 3,
        }
    }
}

/// Which scene a worker spawns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fixture {
    /// `vb_occ_mixed` — eight instances, four of them hidden. Pixel-pinned in four regimes.
    Mixed,
    /// `vb_occ_dense` — the hidden set replicated `K` times. No pin; a host-oracle verdict
    /// cross-check instead.
    Dense,
}

impl Fixture {
    const fn name(self) -> &'static str {
        match self {
            Fixture::Mixed => "mixed",
            Fixture::Dense => "dense",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "mixed" => Fixture::Mixed,
            "dense" => Fixture::Dense,
            other => panic!("`{other}` is not a timing fixture (expected `mixed` or `dense`)"),
        }
    }

    fn from_env() -> Self {
        Self::parse(&std::env::var(ENV_FIXTURE).unwrap_or_else(|_| Fixture::Mixed.name().to_string()))
    }
}

// ===============================================================================================
// The worker
// ===============================================================================================

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let leg = Leg::parse(&std::env::var(ENV_LEG).expect("the timing worker is told its leg"));
    match Fixture::from_env() {
        Fixture::Mixed => vb_occ_mixed_scene::spawn_mixed(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut geo_table,
            &dev,
            leg.marked(),
        ),
        Fixture::Dense => vb_occ_dense::spawn_dense(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut geo_table,
            &dev,
            leg.marked(),
            vb_occ_dense::k_from_env(),
        ),
    }
}

/// **THE WORKER** — one `VisibilityBuffer × Mesh` boot in the leg and the fixture the driver names.
///
/// It serves BOTH drivers: the timing protocol (which arms `BOYKO_VB_ZONE`, or a frame cap for
/// channel W) and the dense oracle gate (which arms the three capture knobs). One worker, because
/// two would be two texts that can disagree about what "the armed leg" boots.
///
/// No capture is armed by the worker itself: a capture changes the frame loop it is being used to
/// time, and holds it open past the frame cap.
#[test]
#[ignore = "needs a real windowed GPU device; the drivers spawn it once per (leg, budget)"]
fn vg_occ_split_timing_worker() {
    if std::env::var(ENV_LEG).is_err() {
        eprintln!(
            "{WORKER}: {ENV_LEG} is unset -- SKIPPED. This worker exists to be spawned by the \
             drivers in this file; booted without a leg it would render forever."
        );
        return;
    }
    // Every driver arms exactly one exit condition. Booted without any, `app.run()` never returns —
    // a hang, the worst failure mode a sweep can have, and the one failure this file's own workers
    // are otherwise structurally exposed to (they arm no capture of their own).
    // WARNING: the ZONE knob, not `BOYKO_VB_BENCH`. Profiling rung 7 step 2 deleted the readback
    // loop that made the bench knob terminate and step 5 deleted its collector, so `BOYKO_VB_BENCH`
    // became an exit condition that does not exit: this guard would pass and `app.run()` would
    // render forever, which is precisely the hang the guard exists to prevent. A stale name in a
    // liveness check is worse than no check, because it answers the question it was asked.
    let has_exit = std::env::var("BOYKO_WINDOW_FRAMES").is_ok()
        || std::env::var("BOYKO_VB_ZONE").is_ok()
        || std::env::var("BOYKO_VB_CULL_READBACK").is_ok();
    if !has_exit {
        eprintln!(
            "{WORKER}: no exit condition is armed (none of BOYKO_WINDOW_FRAMES, BOYKO_VB_ZONE, \
             BOYKO_VB_CULL_READBACK) -- SKIPPED rather than rendering forever."
        );
        return;
    }

    let leg = Leg::parse(&std::env::var(ENV_LEG).expect("invariant: the leg was just checked"));
    let fixture = Fixture::from_env();
    let extent = vb_occ_mixed_scene::EXTENT;
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_engine vg occ timing", extent, extent));
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Mesh,
    });
    // The pyramid is armed on EVERY leg, including the disarmed one, so `vb_hzb_build` differs
    // between legs only in POSITION and never in existence — the same reason `vb_occ_split_gate.rs`
    // arms it on its unmarked control and `[vb_occ_mixed_off]` carries `BOYKO_VG_HZB`.
    app.insert_resource(HzbConfig { mode: HzbMode::Build });
    // VG R3 piece 4 rung P4-4: the OWNER conjunct, through THE single insert site. Since rung P4-6
    // the MODE is a property of the LEG (`Off` on the disarmed one, with the markers still present)
    // and the REGIME comes from the env the driver set, decoded where every other fixture decodes
    // it. The config half of the decode is deliberately dropped: this worker's mode is its leg.
    let (_, force) = occ_fixture::occlusion_from_env();
    occ_fixture::arm_occlusion_with(&mut app, leg.mode(), force);
    if fixture == Fixture::Dense {
        vb_occ_dense::assert_no_split_producer(&app);
    }
    app.run();
}

// ===============================================================================================
// One worker's channel-G summary, parsed
// ===============================================================================================

/// What one `BOYKO_VB_ZONE` worker published about its timed frames.
#[derive(Clone, Debug)]
struct BenchSummary {
    /// Per pass: the worker's own median over its timed frames, in ns.
    median_ns: [f64; PASS_COUNT],
    /// Per pass: the 95th percentile over this worker's timed frames.
    ///
    /// Read for exactly one purpose — [`resolution_of`] turns `p95 − median` into a dispersion and
    /// then into the standard error of the median, which is the band's FLOOR. It is an order
    /// statistic, so it is an actual sample and sits on the timestamp lattice.
    p95_ns: [f64; PASS_COUNT],
    /// Per pass: the median `begin_off_ns`, relative to pair 0's begin.
    begin_off_ns: [f64; PASS_COUNT],
    /// Per pass: the median `end_off_ns`, relative to pair 0's begin — reduced by the runner from
    /// PER-FRAME `(end − base)` values, never composed from the two medians beside it.
    ///
    /// ⚠️ Every clause here that needs an END time reads THIS. Rung P4-6's first sitting computed
    /// it as `begin_off_ns + median_ns` and the `e8 ≤ e9` clause reported the inequality backwards
    /// by 144 ns on a 47 µs run: `median(off) + median(dur) ≠ median(off + dur)` whenever the begin
    /// offset jitters, and it does. The instrument was fixed rather than the clause relaxed.
    end_off_ns: [f64; PASS_COUNT],
    /// Per pass: `Some("FALLBACK")` / `Some("TORN")` when the recorder's witness flagged it.
    flag: [Option<&'static str>; PASS_COUNT],
    /// Per pass: the kept-frame count the worker reported.
    n: [usize; PASS_COUNT],
    /// The `VB-P4 regime` line's `observed=[…]` word list.
    force_words: String,
    /// The `VB-P4 regime` line's `mode=[…]` word list.
    mode_words: String,
    /// The `VB-P4 regime` line's `n_distinct=`.
    n_distinct: usize,
}

impl BenchSummary {
    /// Parses one worker's merged stdout+stderr.
    ///
    /// # Panics
    ///
    /// On any missing line, naming the pass or the key and echoing the worker's whole output. A
    /// worker that completed and published nothing is an instrument failure, not a measurement, and
    /// there is no reduction that can recover from it.
    fn parse(art: &Artifact, output: &str, who: &str) -> Self {
        let mut median_ns = [0.0; PASS_COUNT];
        let mut p95_ns = [0.0; PASS_COUNT];
        let mut begin_off_ns = [0.0; PASS_COUNT];
        let mut end_off_ns = [0.0; PASS_COUNT];
        let mut flag: [Option<&'static str>; PASS_COUNT] = [None; PASS_COUNT];
        let mut n = [0usize; PASS_COUNT];

        // Profiling rung 7: the six per-pass figures come from the artifact's zone rows.
        //
        // `ZONE_BASE_VB` is 0, so a VB pass's slot IS its zone id -- and `PASS_LABELS` was
        // already indexed by slot, so the `pass label -> zone` table this migration was expected to
        // need turned out to be the loop counter it already had. The labels stay, demoted from
        // lookup keys to error-message text.
        for (slot, label) in PASS_LABELS.iter().enumerate() {
            let row = art.zones.iter().find(|z| z.zone as usize == slot).unwrap_or_else(|| {
                panic!(
                    "{who}: the artifact carries no row for zone {slot} (`{label}`). Either the \
                     window never reached its frame budget, or the recorder never bracketed that \
                     pass.\n---- worker output ----\n{output}"
                )
            });
            median_ns[slot] = row.median_ns;
            p95_ns[slot] = row.p95_ns;
            begin_off_ns[slot] = row.begin_off_ns;
            // `end_off_ns` is CARRIED, never `begin + median`: rung P4-6 measured that
            // `median(off) + median(dur) != median(off + dur)` whenever the begin offset jitters,
            // and it does. The artifact carries it for the same reason the printed line did.
            end_off_ns[slot] = row.end_off_ns;
            n[slot] = row.n as usize;
            // The 2x2 label under the two names this harness already knows. `Lost` joins `Torn` on the
            // dominating side: both mean "this row is not a number", which is what the clauses ask.
            flag[slot] = match row.label {
                ZoneLabel::Torn | ZoneLabel::Lost => Some("TORN"),
                ZoneLabel::NotBracketed => Some("FALLBACK"),
                ZoneLabel::Measured => None,
            };
        }

        // The regime provenance is the artifact's header census (rung 7, schema 3). It could not
        // be derived from `workload_tag`: P4-4 made the regime a LIVE Resource, so a boot-frozen
        // value cannot see a mid-run flip, which is the whole thing this triple exists to expose.
        let force_words = art.header.regimes.clone();
        let mode_words = art.header.modes.clone();
        let n_distinct = art.header.regime_n_distinct as usize;

        Self {
            median_ns,
            p95_ns,
            begin_off_ns,
            end_off_ns,
            flag,
            n,
            force_words,
            mode_words,
            n_distinct,
        }
    }
}

// ===============================================================================================
// The statistics
// ===============================================================================================

/// The median of `v` (which must be non-empty and finite).
fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).expect("invariant: samples are finite, never NaN"));
    let n = s.len();
    if n.is_multiple_of(2) { (s[n / 2 - 1] + s[n / 2]) * 0.5 } else { s[n / 2] }
}

/// The p90 of `|v|` — the same index convention the runner's own summary uses for p95.
fn p90_abs(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.iter().map(|x| x.abs()).collect();
    s.sort_by(|a, b| a.partial_cmp(b).expect("invariant: samples are finite, never NaN"));
    s[((s.len() as f64 * 0.90) as usize).min(s.len() - 1)]
}

/// `σ / mean`, sample standard deviation. Stable in `n`, unlike peak-to-peak — the correction
/// `vg_decidability_floor.rs` measured against its own first design.
fn cv(v: &[f64]) -> f64 {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    if mean <= 0.0 || v.len() < 2 {
        return f64::INFINITY;
    }
    let var = v.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / (n - 1.0);
    var.sqrt() / mean
}

/// The relative difference of two medians, signed: positive means `b` is SLOWER than `a`.
fn relative(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (median(a), median(b));
    if ma <= 0.0 { f64::INFINITY } else { (mb - ma) / ma }
}

/// **THE TWIN TERM** of the band rule: `max(|median|, p90|·|)` over a quantity's zero control.
///
/// Both halves are properties of THIS sitting. Nothing is carried in from a previous run, because
/// the floor is MEASURED not to be a constant.
///
/// ⚠️ This is a **DRIFT** estimator and nothing else — see [`Quantity::band`] for the plan-level
/// defect that discovery exposed, and for the resolution term that now sits beside it.
fn band_of(zero_twin: &[f64]) -> f64 {
    median(zero_twin).abs().max(p90_abs(zero_twin))
}

/// **THE RESOLUTION of ONE published median**, in ns — the band's floor, per contributing term.
///
/// The runner publishes a `median`, a `p95` and an `n` for every pass of every worker. For a
/// roughly normal bulk, `p95 − median ≈ z·σ` with `z` = [`Z95`], and the standard error of a MEDIAN
/// is `√(π/2)·σ/√n` ([`MEDIAN_SE_FACTOR`]). That is how finely this instrument can place the number
/// it actually published, computed from the very samples that median reduced, in this sitting.
///
/// # ⚠️ Why `q` is a SUB-floor and not a fudge
///
/// A perfectly reproducible pass reports `p95 == median`, hence `σ̂ = 0`, hence `SE = 0` — and a
/// floor of zero is the degenerate band this whole mechanism exists to prevent. It would be
/// claiming resolution finer than the counter can represent. `q` is the timestamp lattice measured
/// in the same sitting ([`measured_quantum_ns`]); it binds ONLY where the observed dispersion is
/// below one tick, which is a statement no real instrument can support. It is not the floor — the
/// dispersion is; it is the point below which the dispersion stops being believable.
fn resolution_of(row: &BenchSummary, p: usize, q: f64) -> f64 {
    let sigma = (row.p95_ns[p] - row.median_ns[p]).max(0.0) / Z95;
    let se = MEDIAN_SE_FACTOR * sigma / (row.n[p].max(1) as f64).sqrt();
    se.max(q)
}

/// The resolution floor of a contrast between `base` and `arm` over the passes `ps`, all of which
/// enter with unit coefficient: `Σ_p [ res(base, p) + res(arm, p) ]`.
///
/// Every median a quantity is built from contributes its own resolution, because a difference of
/// two numbers is no better placed than the two numbers are. Summing rather than combining in
/// quadrature is the conservative choice: the per-leg errors are not independent (one GPU, one
/// sitting), and a band that is too wide refuses a real result while a band that is too narrow
/// manufactures one.
fn floor_over(base: &BenchSummary, arm: &BenchSummary, ps: &[usize], q: f64) -> f64 {
    ps.iter().map(|&p| resolution_of(base, p, q) + resolution_of(arm, p, q)).sum()
}

/// **THE TIMESTAMP LATTICE, MEASURED** — the GCD of every timestamp-derived value this sitting
/// published, in ns.
///
/// Every `median_ns`, `p95_ns`, `begin_off_ns` and `end_off_ns` is an order statistic of raw tick
/// deltas scaled by the device's `timestampPeriod`, so every one of them is an exact multiple of
/// the counter's increment. Their GCD is therefore that increment — measured on this device, in
/// this sitting, rather than queried or assumed.
///
/// ⚠️ **`VkPhysicalDeviceLimits::timestampPeriod` is NOT this number.** It is the tick→ns SCALE
/// (`1.0` on the vendor this rung was authored against) and says nothing about how often the
/// counter increments — **32 ns** in the sitting this text describes, under the odd
/// [`DEFAULT_BENCH_FRAMES`]. (An earlier text said 16 ns; that GCD was taken over medians from an
/// EVEN budget, each the mean of two middle samples, so it could legitimately read `q/2`.) The value
/// is re-measured every sitting and hard-coded nowhere. Flooring a band at `period × 1 tick` would
/// satisfy every assertion here while under-stating the instrument's resolution by the whole lattice
/// factor — 32× as measured — which silences the alarm and leaves the false win, the reason that
/// route was refused.
///
/// `mean_ns` is deliberately excluded: an arithmetic mean of `n` lattice values is not itself on
/// the lattice. The medians are only on it because [`DEFAULT_BENCH_FRAMES`] is odd.
///
/// Works in tenths of a nanosecond — the full precision the summary prints — so a half-tick value
/// from an even-budget worker degrades the answer honestly (to `q/2`) instead of corrupting it.
/// Returns `0.0` if the sitting published no nonzero value at all, which the band's own
/// non-vacuity assertion then refuses by name.
fn measured_quantum_ns(gpu: &[Vec<BenchSummary>; 4]) -> f64 {
    let mut g: u64 = 0;
    for leg in gpu {
        for row in leg {
            for p in 0..PASS_COUNT {
                for v in [row.median_ns[p], row.p95_ns[p], row.begin_off_ns[p], row.end_off_ns[p]] {
                    let t = (v * 10.0).round();
                    if t > 0.0 {
                        g = gcd_u64(g, t as u64);
                    }
                }
            }
        }
    }
    g as f64 / 10.0
}

/// Binary-free Euclid, iterative so a long sample set cannot deepen the stack.
fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// The seven contrast quantities derivable from one `(base, armed)` pair of workers.
///
/// Called twice per round with the SAME arithmetic: `(B, C)` for the real reading and `(A0, A1)`
/// for the zero twin. One function, so the band cannot be computed by a different reduction than
/// the number it bounds.
#[derive(Clone, Copy, Debug)]
struct Contrast {
    net_run: f64,
    saving: f64,
    overhead: f64,
    net: f64,
    gap_residual: f64,
    hzb_residual: f64,
    residual: f64,
}

fn contrast(base: &BenchSummary, arm: &BenchSummary) -> Contrast {
    let d = |p: usize| arm.median_ns[p] - base.median_ns[p];
    let net_run = d(P_RUN);
    let saving = -d(P_EARLY_RASTER);
    let overhead = d(P_LATE_UPLOAD) + d(P_EARLY_CULL) + d(P_LATE_CULL) + d(P_LATE_RASTER);
    let hzb_residual = d(P_HZB_BUILD);
    let net = overhead + hzb_residual - saving;
    Contrast {
        net_run,
        saving,
        overhead,
        net,
        gap_residual: net_run - net,
        hzb_residual,
        residual: d(P_CULL_RESET) + d(P_CULL_DISPATCH) + d(P_VB_SHADE),
    }
}

/// `Bracketed`: `Σ_{p ∈ 0..=8, p ≠ 6} [m_p(arm) − m_p(base)]`.
///
/// Slot 6 is excluded because its recorder site MOVES between an armed and a disarmed leg, and slot
/// 9 is reported separately because it CONTAINS slots 3..8 — summing it in would double-count.
fn bracketed(base: &BenchSummary, arm: &BenchSummary) -> f64 {
    (0..=P_LATE_RASTER)
        .filter(|&p| p != P_HZB_BUILD)
        .map(|p| arm.median_ns[p] - base.median_ns[p])
        .sum()
}

/// One published quantity: its per-round readings, its per-round zero twin, its per-round
/// resolution floor, and whether its SIGN is the claim (which is what decides whether a band is
/// claimed for it at all).
struct Quantity {
    name: &'static str,
    real: Vec<f64>,
    zero: Vec<f64>,
    /// Per round, the propagated resolution of the medians this quantity is built from
    /// ([`floor_over`]). Computed on the SAME legs the real reading uses — the floor bounds how
    /// finely the READING can be placed, not how finely the control can.
    floor: Vec<f64>,
    /// The one-line meaning of a positive value, printed beside every reading.
    positive_means: &'static str,
}

impl Quantity {
    /// **THE BAND RULE**: `max(resolution floor, zero-twin drift)`.
    ///
    /// # ⚠️ PLAN-LEVEL FINDING against `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md` §C2
    ///
    /// §C2 defines the band as the zero twin ALONE: *"the band comes from running the identical
    /// reduction on the zero control"*. That is a **DRIFT** estimator and only a drift estimator.
    /// `A0` and `A1` are the SAME configuration — same scene, same env (byte-identical, by design),
    /// same command stream — measured on a fully serialized frame (`wait_idle` per bench frame on
    /// top of unconditional FIFO). On a deterministic GPU the twin's expected value is therefore
    /// **exactly zero**, and rung P4-6's third sitting measured exactly that: `band(NetRun) = 0`
    /// across five rounds of ten separate processes, with `m_9` a perfectly healthy 47 104 ns
    /// (`vb_occ_mixed`) / 691 200 ns (`vb_occ_dense`).
    ///
    /// With drift at zero the verdict rule collapses from *"|Q| clears the instrument's noise"* to
    /// **"Q ≠ 0"** — which is how a false win is manufactured, and why `Saving` read `RESOLVED` on
    /// eight low-poly instances at 512².
    ///
    /// **The zero control is not at fault and is not removed.** It did its job: it proved there is
    /// no process-to-process drift on the GPU channel (channel W's own legs wandered 6–9 % in the
    /// same sitting, which is the clean discriminator). What it structurally cannot supply is
    /// RESOLUTION — the other half of what a verdict needs. §C2 conflated the two. **Rung P4-7
    /// LANDED the plan's repair**: §C2 now defines `band(Q) = max(FLOOR, TWIN)` and carries this
    /// finding as the reason, so the plan and this code no longer say different things.
    ///
    /// Both components are published beside every quantity, because a band that is all-floor and a
    /// band that is all-twin mean different things and a future degenerate twin must stay visible.
    fn band(&self) -> f64 {
        self.floor_term().max(self.twin_term())
    }

    /// The resolution half of the band — see [`resolution_of`].
    fn floor_term(&self) -> f64 {
        median(&self.floor)
    }

    /// The drift half of the band — see [`band_of`].
    fn twin_term(&self) -> f64 {
        band_of(&self.zero)
    }

    fn value(&self) -> f64 {
        median(&self.real)
    }

    /// `RESOLVED` iff the reading clears its own sitting's band.
    fn verdict(&self) -> &'static str {
        if self.value().abs() > self.band() { "RESOLVED" } else { "NOT RESOLVED" }
    }
}

// ===============================================================================================
// Driving the worker
// ===============================================================================================

/// The env every worker gets, whatever the channel: the leg, the fixture, and the removal of every
/// knob an operator's shell might be carrying.
///
/// The removals are as load-bearing as the settings. `BOYKO_VB_CULL_READBACK` in particular MUST
/// go on the bench legs: the runner refuses the probe and the bench together at boot, because the
/// probe records buffer copies INSIDE `VbEarlyCull`'s bracket and immediately after `VbLateRaster`'s
/// and `VbRun`'s end stamps. That refusal is what makes "no published number contains any part of
/// the probe's cost" a structural statement rather than a footnote.
fn base_worker_cmd(fixture: Fixture, leg: Leg, k: u32) -> Command {
    let exe = std::env::current_exe().expect("invariant: the test binary knows its own path");
    let mut cmd = Command::new(exe);
    cmd.args([WORKER, "--ignored", "--exact", "--test-threads=1"])
        .env(ENV_LEG, leg.name())
        .env(ENV_FIXTURE, fixture.name())
        .env(vb_occ_dense::ENV_K, k.to_string())
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS")
        .env_remove("BOYKO_HZB_DUMP")
        .env_remove("BOYKO_VB_PROBE")
        .env_remove("BOYKO_VB_CULL_READBACK")
        .env_remove("BOYKO_VB_BENCH")
        .env_remove("BOYKO_SV0_BENCH")
        .env_remove("BOYKO_SV0_BENCH_NULL")
        // Bench-shape knobs from a shell would change the scene and the printed label; neither is
        // asserted, but a worker whose scene depends on the ambient environment is not reproducible.
        .env_remove("BOYKO_VB_BENCH_LIGHTS")
        .env_remove("BOYKO_VB_BENCH_GRID")
        .env_remove("BOYKO_VB_BENCH_RIG")
        .env_remove("BOYKO_VB_FROXEL_FORCE_OFF")
        // The marker predicate: this protocol's markers come from the leg, never from the env, and
        // a stray `BOYKO_VG_OCC` would only feed `occlusion_from_env`'s config half, which the
        // worker drops. Removed anyway, so no reader has to reconstruct that argument.
        .env_remove(occ_fixture::ENV_OCC)
        // The regime is the ONE variable between B and the other three legs. Removed rather than
        // left inherited, so a stray shell value cannot silently make the real leg a forced one.
        .env_remove(occ_fixture::ENV_OCC_FORCE);
    if let Some(f) = leg.force() {
        cmd.env(occ_fixture::ENV_OCC_FORCE, f);
    }
    cmd
}

/// Runs ONE worker process for channel W and returns its wall clock, in microseconds.
fn wall_clock_us(fixture: Fixture, leg: Leg, k: u32, frames: u32) -> f64 {
    let mut cmd = base_worker_cmd(fixture, leg, k);
    cmd.env("BOYKO_WINDOW_FRAMES", frames.to_string());
    let t0 = Instant::now();
    let status = cmd.status().expect("invariant: the timing worker spawns");
    let dt = t0.elapsed();
    assert!(
        status.success(),
        "the timing worker (`{}`, {frames} frames, {}) exited {status}",
        leg.name(),
        fixture.name()
    );
    dt.as_secs_f64() * 1.0e6
}

/// Channel W's per-frame period for one leg, two-point subtracted.
///
/// The subtraction cancels the constant boot cost — device creation, window creation, shader
/// modules, the first-frame pipeline warm-up — which on a windowed boot dwarfs the frames
/// themselves and which no single-budget measurement can separate out.
fn per_frame_us(fixture: Fixture, leg: Leg, k: u32, long_frames: u32) -> f64 {
    let span = f64::from(long_frames - SHORT_FRAMES);
    (wall_clock_us(fixture, leg, k, long_frames) - wall_clock_us(fixture, leg, k, SHORT_FRAMES))
        / span
}

/// Runs ONE worker process with the shipped `BOYKO_VB_ZONE` recorder armed and returns its
/// ten-pass summary, or `None` if this device serves no timestamps at all.
///
/// The bench returns on its own once it reaches its frame budget, so no frame cap is set. Both
/// output streams are merged: the `VB-P4`/`VB-P1d` lines are `println!` while the runner's scope
/// notes and panics are `eprintln!`, and no clause below cares which stream carried its evidence.
fn bench_summary(fixture: Fixture, leg: Leg, k: u32, bench_frames: u32) -> Option<BenchSummary> {
    // One file per worker, chosen and deleted by the PARENT: this driver spawns many children and
    // a shared path is a stale-read generator (`artifact.rs`'s Decision 2/3).
    let mut artifact = std::env::temp_dir();
    artifact.push(format!("boyko_vg_occ_{}_{}_k{k}.toml", fixture.name(), leg.name()));
    let _ = std::fs::remove_file(&artifact);
    let token = format!("vg-occ-{}-{}-k{k}", fixture.name(), leg.name());
    let mut cmd = base_worker_cmd(fixture, leg, k);
    cmd.args(["--nocapture"])
        // Profiling rung 7: the ZONE recorder, and a per-worker artifact the parent names and
        // stamps. The retired knob is removed
        // rather than assumed unset -- an operator's stale shell variable would otherwise fail
        // every worker at boot with a message about a configuration this driver never asked for.
        .env("BOYKO_VB_ZONE", "1")
        .env_remove("BOYKO_VB_BENCH")
        .env("BOYKO_PROFILE_ARTIFACT", &artifact)
        .env("BOYKO_PROFILE_RUN_TOKEN", &token)
        .env("BOYKO_PROFILE_WORKLOAD", format!("{}_{}_k{k}", fixture.name(), leg.name()))
        .env("BOYKO_VB_BENCH_FRAMES", bench_frames.to_string())
        .env_remove("BOYKO_WINDOW_FRAMES");
    let out = cmd.output().expect("invariant: the timing worker spawns");
    let mut merged = String::from_utf8_lossy(&out.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&out.stderr));
    if merged.contains(NO_TIMESTAMPS) {
        return None;
    }
    let who = format!("channel G `{}` on `{}`", leg.name(), fixture.name());
    assert!(
        out.status.success(),
        "{who}: the bench worker exited {}.\n---- worker output ----\n{merged}",
        out.status
    );
    // Read with the token the parent chose: a leftover from an earlier worker is refused on the
    // header rather than folded into this one's numbers.
    let art = Artifact::read(&artifact, &token).unwrap_or_else(|e| {
        panic!("{who}: the worker completed but its artifact is unusable: {e}
---- worker output ----
{merged}")
    });
    let _ = std::fs::remove_file(&artifact);
    Some(BenchSummary::parse(&art, &merged, &who))
}

// ===============================================================================================
// The measurement
// ===============================================================================================

/// **`vb_occ_mixed`** — the pixel-pinned fixture, eight instances.
#[test]
#[ignore = "live GPU measurement (spawns 60 windowed workers); the orchestrator runs it with --test-threads=1"]
fn vg_occ_split_timing_mixed() {
    run_protocol(Fixture::Mixed);
}

/// **`vb_occ_dense`** — the same geometry with the hidden set replicated `K` times. No pin.
#[test]
#[ignore = "live GPU measurement (spawns 60 windowed workers); the orchestrator runs it with --test-threads=1"]
fn vg_occ_split_timing_dense() {
    run_protocol(Fixture::Dense);
}

/// The whole protocol on one fixture: the prediction, the rounds, the report, the assertions.
///
/// `#[allow(clippy::too_many_lines)]`: this is a measurement PROTOCOL, and its steps are ordered by
/// the protocol rather than by decomposition. Splitting it would put the prediction, the interleave
/// and the assertions in three places that can each drift from what the report claims — the one
/// coupling this rung exists to keep visible.
#[allow(clippy::too_many_lines)]
fn run_protocol(fixture: Fixture) {
    let rounds: usize = env_or("BOYKO_VG_OCC_TIMING_ROUNDS", DEFAULT_ROUNDS);
    let long_frames: u32 = env_or("BOYKO_VG_OCC_TIMING_FRAMES", DEFAULT_LONG_FRAMES);
    let bench_frames: u32 = env_or("BOYKO_VG_OCC_TIMING_BENCH_FRAMES", DEFAULT_BENCH_FRAMES);
    let k = vb_occ_dense::k_from_env();
    assert!(
        long_frames > SHORT_FRAMES,
        "the two-point subtraction needs N2 ({long_frames}) > N1 ({SHORT_FRAMES})"
    );
    assert!(
        rounds > 0,
        "every band below is a reduction over ROUNDS of the zero control; at zero rounds there is \
         no control, and a report with no control is the failure this protocol exists to avoid"
    );
    if fixture == Fixture::Dense {
        // The fixture's own arithmetic, checked before 60 GPU processes are spent on it.
        vb_occ_dense::assert_fixture_invariants(k);
    }

    print_preamble(fixture, k, rounds, long_frames, bench_frames);

    // ---- the interleave: A0 -> B -> C -> A1, both channels at each leg's position ---------------
    let mut wall: [Vec<f64>; 4] = core::array::from_fn(|_| Vec::with_capacity(rounds));
    let mut gpu: [Vec<BenchSummary>; 4] = core::array::from_fn(|_| Vec::with_capacity(rounds));
    for r in 0..rounds {
        for slot in SLOTS {
            wall[slot.idx()].push(per_frame_us(fixture, slot.leg(), k, long_frames));
            match bench_summary(fixture, slot.leg(), k, bench_frames) {
                Some(s) => gpu[slot.idx()].push(s),
                None => {
                    eprintln!(
                        "VG R3 P4-6: INSTRUMENT-DEAD -- this device reports unusable timestamps, \
                         so BOYKO_VB_ZONE arms no recorder and channel G does not exist here. \
                         Channel W is KNOWN-BLIND by itself, so this sitting produces no number. \
                         Re-run on a timestamp-capable device."
                    );
                    return;
                }
            }
        }
        eprintln!(
            "VG R3 P4-6 [{}]: round {} of {rounds} -- W us/frame A0={:.1} B={:.1} C={:.1} A1={:.1}",
            fixture.name(),
            r + 1,
            wall[0][r],
            wall[1][r],
            wall[2][r],
            wall[3][r]
        );
    }

    // ---- clause 1: totality and structural health, BEFORE any reduction -------------------------
    for slot in SLOTS {
        let rows = &gpu[slot.idx()];
        assert_eq!(
            rows.len(),
            rounds,
            "`{}` produced {} of {rounds} channel-G samples. Pooling only the survivors would \
             UNDERSTATE the spread, which is the one quantity every band is computed from.",
            slot.label(),
            rows.len()
        );
        for (r, row) in rows.iter().enumerate() {
            for (p, label) in PASS_LABELS.iter().enumerate() {
                assert!(
                    row.flag[p].is_none(),
                    "`{}` round {}: pass `{label}` came back {} -- it measured NOTHING (a frame-end \
                     zero pair from the totality epilogue) or was torn. Every leg of this protocol \
                     is a `VisibilityBuffer x Mesh` boot with the pyramid armed, so all ten brackets \
                     must execute; a flag here is a recorder or a fixture defect, and averaging it \
                     in would publish a fabricated ~0 as a cost.",
                    slot.label(),
                    r + 1,
                    row.flag[p].expect("invariant: just matched Some")
                );
                // BOTH offsets: they share one base stamp, so the contract is one contract.
                assert!(
                    row.begin_off_ns[p] < MAX_BEGIN_OFF_NS
                        && row.end_off_ns[p] < MAX_BEGIN_OFF_NS,
                    "`{}` round {}: pass `{label}` reports begin_off_ns={:.1} end_off_ns={:.1}, \
                     past the {MAX_BEGIN_OFF_NS:.0} ns base-stamp bound. Both are relative to pair \
                     0's begin; a broken base reads as ~2^36 ticks, not as a plausible number.",
                    slot.label(),
                    r + 1,
                    row.begin_off_ns[p],
                    row.end_off_ns[p]
                );
                assert_eq!(
                    row.n[p], bench_frames as usize,
                    "`{}` round {}: pass `{label}` reports n={} kept frames against a budget of \
                     {bench_frames}. Every row grows together -- one push per pass per kept frame -- \
                     so a short row means the collector dropped a pass on some frames and the \
                     per-pass medians below are reductions over DIFFERENT frame sets.",
                    slot.label(),
                    r + 1,
                    row.n[p]
                );
            }
            // ---- clause 2: the regime this worker actually ran ---------------------------------
            assert_eq!(
                row.n_distinct,
                1,
                "`{}` round {}: the worker observed {} distinct occlusion regimes across its timed \
                 frames (observed=[{}] mode=[{}]). A worker that changed regime mid-run is REJECTED \
                 rather than averaged -- two regimes attributed to one number is exactly the \
                 provenance failure the `VB-P4 regime` line exists to make visible.",
                slot.label(),
                r + 1,
                row.n_distinct,
                row.force_words,
                row.mode_words
            );
            assert_eq!(
                row.force_words,
                slot.leg().expected_force_word(),
                "`{}` round {}: the worker ran regime `{}`, not `{}`",
                slot.label(),
                r + 1,
                row.force_words,
                slot.leg().expected_force_word()
            );
            assert_eq!(
                row.mode_words,
                slot.leg().expected_mode_word(),
                "`{}` round {}: the worker ran mode `{}`, not `{}`. The disarmed legs are \
                 `OcclusionMode::Off` WITH markers since rung P4-6; a `two_phase` here means the \
                 one-variable contrast collapsed.",
                slot.label(),
                r + 1,
                row.mode_words,
                slot.leg().expected_mode_word()
            );
            // ---- clauses 3 and 4 ---------------------------------------------------------------
            //
            // CLAUSE 9 WAS HERE AND IS DELETED, because profiling rung 7 deleted its subject.
            //
            // It compared the `VB-P1d` line's shade mean against `VB-P4 pass=vb_shade`'s — TWO
            // PRINTERS over one sample row — so that P4-1's byte-identity guarantee for the VB-P1d
            // line kept meaning something "while something still reads it". Rung 7 deleted both
            // printers and moved every figure here onto the artifact. One operand of the
            // comparison no longer exists, and a clause with one operand is not a weaker clause,
            // it is not a clause.
            //
            // This is the disposition rung 7 already recorded for `vb_bench_totality_gate.rs` —
            // "a file whose every gate has lost its subject has nothing to migrate" — applied to
            // one clause instead of a whole file.
            //
            // ⚠️ It survived rung 7 because the corpus marked THIS FILE "MIGRATED" on the strength
            // of its other clauses, and because both tests that reach this code are `#[ignore]`d
            // GPU orchestrators, so no sweep has ever run it. The failure it would have produced
            // blames the worker — "the worker printed no `VB-P1d ` line" — for something rung 7
            // did to that worker on purpose.
            assert_record_order(row, slot, r + 1);
        }
    }

    // ---- the quantities, and their zero twins ---------------------------------------------------
    let mut net_run = qty("NetRun", "the decision COSTS more than it saves");
    let mut saving = qty("Saving", "the decision SHRINKS the early raster");
    let mut gap_res = qty("GapResidual", "the partition identity does not close");
    let mut hzb_res = qty("HzbResidual", "the pyramid build is leg-dependent");
    let mut residual = qty("Residual", "the instrument is contaminated");
    let mut brack = qty("Bracketed", "the bracketed ranges cost more armed");
    let (mut overhead, mut net, mut plumb, mut late_share) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // `Δ4` is a SCALE, never a significance verdict, so it is not in `banded` and carries no
    // assertion — but the `ALSO:` line below compares it against a threshold, and until rung P4-6
    // that threshold was the bare zero twin. A zero twin makes "clears its spread" trivially true,
    // which is the same defect §C2 carried. It gets the same `max(floor, twin)` band as everything
    // else that is compared against anything.
    let mut d4 = qty("D_4 vb_early_cull", "the occlusion LEAF costs more with the decision armed");
    // One row per ROUND, carrying that round's four workers in slot order. Materialised rather than
    // indexed in the loop below so every quantity is derived from the four legs of ONE round and
    // cannot silently pair a `C` from round 3 with a `B` from round 4.
    let by_round: Vec<[&BenchSummary; 4]> =
        (0..rounds).map(|r| [&gpu[0][r], &gpu[1][r], &gpu[2][r], &gpu[3][r]]).collect();
    // The lattice, measured off everything this sitting published — the SUB-floor inside each
    // quantity's resolution term. Read `measured_quantum_ns`'s doc before touching it; in
    // particular it is NOT `VkPhysicalDeviceLimits::timestampPeriod`, and that is load-bearing.
    let quantum = measured_quantum_ns(&gpu);
    for [a0, b, c, a1] in by_round {
        let real = contrast(b, c);
        let zero = contrast(a0, a1);
        net_run.real.push(real.net_run);
        net_run.zero.push(zero.net_run);
        net_run.floor.push(floor_over(b, c, &[P_RUN], quantum));
        saving.real.push(real.saving);
        saving.zero.push(zero.saving);
        saving.floor.push(floor_over(b, c, &[P_EARLY_RASTER], quantum));
        gap_res.real.push(real.gap_residual);
        gap_res.zero.push(zero.gap_residual);
        // `GapResidual = Δ9 − (Δ3+Δ4+Δ5+Δ6+Δ7+Δ8)`: seven deltas, fourteen medians.
        gap_res.floor.push(floor_over(
            b,
            c,
            &[P_LATE_UPLOAD, P_EARLY_CULL, P_EARLY_RASTER, P_HZB_BUILD, P_LATE_CULL, P_LATE_RASTER, P_RUN],
            quantum,
        ));
        hzb_res.real.push(real.hzb_residual);
        hzb_res.zero.push(zero.hzb_residual);
        hzb_res.floor.push(floor_over(b, c, &[P_HZB_BUILD], quantum));
        residual.real.push(real.residual);
        residual.zero.push(zero.residual);
        residual.floor.push(floor_over(b, c, &[P_CULL_RESET, P_CULL_DISPATCH, P_VB_SHADE], quantum));
        brack.real.push(bracketed(a0, c));
        brack.zero.push(bracketed(a0, a1));
        // `Bracketed`'s legs are A0 and C, so its floor is computed on A0 and C — a floor bounds
        // the reading it accompanies, never the control.
        brack.floor.push(floor_over(
            a0,
            c,
            &(0..=P_LATE_RASTER).filter(|&p| p != P_HZB_BUILD).collect::<Vec<_>>(),
            quantum,
        ));
        overhead.push(real.overhead);
        net.push(real.net);
        // `PlumbRun` removes slot 6 from B's run because on A0 that site is OUTSIDE the run
        // entirely. Attribution-grade: removing an interval from inside a partition cannot undo
        // migration into or out of it.
        plumb.push((b.median_ns[P_RUN] - b.median_ns[P_HZB_BUILD]) - a0.median_ns[P_RUN]);
        late_share.push(real.overhead / a0.median_ns[P_EARLY_RASTER]);
        d4.real.push(c.median_ns[P_EARLY_CULL] - b.median_ns[P_EARLY_CULL]);
        d4.zero.push(a1.median_ns[P_EARLY_CULL] - a0.median_ns[P_EARLY_CULL]);
        d4.floor.push(floor_over(b, c, &[P_EARLY_CULL], quantum));
    }

    // ---- the report ------------------------------------------------------------------------------
    print_channel_w(fixture, &wall, long_frames, rounds);
    print_channel_g(fixture, &gpu, rounds);
    let banded = [&net_run, &saving, &gap_res, &hzb_res, &residual, &brack];
    print_quantities(&banded, &overhead, &net, &plumb, &late_share, &d4, quantum);
    print_observations(&gpu);
    print_decision(&net_run, &saving, &brack, &late_share, &d4);
    print_prediction_outcome(fixture, &saving, &overhead);

    // ---- the remaining assertions ---------------------------------------------------------------
    //
    // Clause 6: a band of zero would make every nonzero reading trivially RESOLVED. Since rung
    // P4-6's band carries a RESOLUTION floor beside the drift twin, a zero band now means the floor
    // ALSO came out zero -- i.e. the dispersion was zero AND the measured lattice was zero -- which
    // is an instrument that published no scale at all, not an instrument with no noise.
    for q in banded {
        assert!(
            q.band() > 0.0,
            "`{}`'s band is EXACTLY 0 (floor={:.1}, twin={:.1}, measured lattice={quantum:.1} ns). \
             A zero band makes every nonzero reading trivially RESOLVED, which is how a false win is \
             manufactured.\n\
             A zero TWIN alone is expected and is not a failure: A0 and A1 are the same \
             configuration on a serialized deterministic GPU. A zero FLOOR means every worker \
             reported `p95 == median` AND the lattice measured 0 -- so the sitting published no \
             resolution scale whatsoever. Check that `p95_ns` and the offsets are real numbers \
             before reading anything else in this report.",
            q.name,
            q.floor_term(),
            q.twin_term()
        );
    }
    let w_control = relative(&wall[0], &wall[3]).abs();
    assert!(
        w_control > 0.0,
        "channel W's zero control is EXACTLY 0.00% across {rounds} pairs of separate processes."
    );

    // Clause 7: a bracket around an empty block must read smaller than one around real work. This
    // is the single clause that decides whether the timestamp channel resolves at this magnitude.
    for (p, what) in [(P_LATE_RASTER, "the late RASTER scope"), (P_LATE_CULL, "the late CULL")] {
        let a0 = median(&gpu[0].iter().map(|s| s.median_ns[p]).collect::<Vec<_>>());
        let b = median(&gpu[1].iter().map(|s| s.median_ns[p]).collect::<Vec<_>>());
        assert!(
            a0 < b,
            "NON-VACUITY: `{}` reads {a0:.1} ns on the DISARMED leg and {b:.1} ns on FORCE-KEEP. \
             On A0 that bracket encloses an EMPTY `if occlusion_split` block; on B it encloses {what}. \
             A zero-width bracket that does not read smaller than one containing real work means \
             the timestamp channel does not resolve at this magnitude, and EVERY NUMBER IN THIS \
             REPORT IS NOISE.",
            PASS_LABELS[p]
        );
    }

    // Clause 8: the three checked residuals.
    for q in [&residual, &gap_res, &hzb_res] {
        assert!(
            q.value().abs() <= q.band(),
            "`{}` reads {:+.1} ns against a band of {:.1} ns.\n{}",
            q.name,
            q.value(),
            q.band(),
            residual_diagnosis(q.name)
        );
    }
}

/// A fresh, empty [`Quantity`].
fn qty(name: &'static str, positive_means: &'static str) -> Quantity {
    Quantity { name, real: Vec::new(), zero: Vec::new(), floor: Vec::new(), positive_means }
}

/// The env value at `key`, or `default`.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// What a residual over its band means, quoted from the decision table so a red carries its own
/// consequence instead of sending the reader to a document.
fn residual_diagnosis(name: &str) -> &'static str {
    match name {
        "Residual" => {
            "THE INSTRUMENT IS CONTAMINATED. Slots 0/1/2 carry no split-dependent work, so a \
             difference between B and C there is not attributable to the decision. Every number in \
             this report is suspect. First suspect: a leg-dependent neighbourhood of `VbShade`. \
             Reported, not worked around."
        }
        "GapResidual" => {
            "THE PARTITION IDENTITY DOES NOT HOLD AS DERIVED. Either an unaccounted gap command \
             between the run's inner brackets is leg-dependent, or a stamp is not where the plan \
             thinks it is. Per-slot numbers become attribution-only; `NetRun` survives, because it \
             is measured directly rather than summed."
        }
        _ => {
            "THE PYRAMID BUILD IS LEG-DEPENDENT, or migration across slot 6 is large. B and C are \
             both split legs, so `record_hzb_poison_build` is called from the same site with the \
             same dispatch chain and the same pyramid extent on both. `Net`'s decomposition is \
             suspect; `NetRun` survives."
        }
    }
}

/// One adjacent pair of `BOTTOM_OF_PIPE` BEGIN stamps in the leg-independent run, carrying the
/// recorder audit that decides its relation.
///
/// See [`BEGIN_CHAIN`] for the audit itself and for why the answer is per PAIR rather than one rule.
struct BeginPair {
    earlier: usize,
    later: usize,
    /// What the recorder puts between the two stamps, quoted from `vb.rs` with its site.
    between: &'static str,
    /// `true` iff EVERY leg of this protocol records GPU work between the two stamps. When it is
    /// `false`, EQUAL offsets are a correct reading and not a defect.
    margin_on_every_leg: bool,
}

/// **THE ADJACENT-STAMP AUDIT.** Each row is a consecutive pair of BEGIN stamps in the span that is
/// identical on all four legs (`b9, b3, b4, b5, b7, b8`), with the answer to the only question that
/// decides the relation: *does the recorder put a GPU command between them?*
///
/// # Why every row asserts `<=`, and why that is not a tolerance
///
/// A `BOTTOM_OF_PIPE` stamp reports a PREFIX-COMPLETION time. Two such stamps with no command
/// between them wait on prefixes that differ by nothing, so their readings are only guaranteed to
/// differ by the timestamp counter's own **lattice quantum** ([`measured_quantum_ns`] — 32 ns in the
/// sitting this text describes, re-measured every sitting). Such pairs came back EQUAL here: the
/// counter did not tick between them. That observed difference is `0` on this machine and legally
/// non-zero elsewhere — the same property `vb_bench_totality_gate.rs:90-101` states for its
/// `FALLBACK_MAX_NS` bound, and for the same reason: pinning one driver's quantum into a gate reds a
/// correct engine somewhere else. `<=` is therefore the TRUE relation for such a pair, not a relaxed
/// one.
///
/// ⚠️ Rung P4-6's first sitting asserted `off(b9) < off(b3)` STRICTLY and both fixtures reported
/// them EQUAL, deterministically, on the disarmed leg — `vb.rs:1598-1599` stamps the two on
/// adjacent lines of one block with nothing between. The strictness was never available.
///
/// # ⚠️ What a violation of a row licenses, and what it does NOT
///
/// It licenses: *a stamp is not at the pipeline stage this harness assumes, or it is not at the
/// recorder site this harness assumes.* It does **not** license any conclusion about the HOST's
/// record ORDER. Record order is a host property; timestamps are device completion times and cannot
/// witness it. The witness for record order is a host-side flag set at the write and read at the
/// dependent site — rung P4-3's `cull_uniform_filled` pattern (`vb.rs:1613`/`:1621`) — and any
/// future clause that wants to claim record order must be built that way instead of read off these
/// numbers.
const BEGIN_CHAIN: [BeginPair; 5] = [
    BeginPair {
        earlier: P_RUN,
        later: P_LATE_UPLOAD,
        between: "NOTHING -- `vb.rs:1598-1599` stamps b9 and b3 on adjacent lines of one block",
        margin_on_every_leg: false,
    },
    BeginPair {
        earlier: P_LATE_UPLOAD,
        later: P_EARLY_CULL,
        between: "slot 3's bracket, holding the `if occlusion_split` late-record fill \
                  (`vb.rs:1617-1735`), plus the EMPTY `e3 -> b4` gap (`vb.rs:1743-1744`). The fill \
                  is skipped on the two DISARMED legs, which therefore have no margin here",
        margin_on_every_leg: false,
    },
    BeginPair {
        earlier: P_EARLY_CULL,
        later: P_EARLY_RASTER,
        between: "slot 4's bracket, holding the `if batch_cull_armed` early cull dispatch \
                  (`vb.rs:1756-2108`), plus the EMPTY `e4 -> b5` gap (`vb.rs:2109-2110`). \
                  `batch_cull_armed` (`vb.rs:1358-1363`) does not depend on the split, so this \
                  dispatch is recorded on all four legs",
        margin_on_every_leg: true,
    },
    BeginPair {
        earlier: P_EARLY_RASTER,
        later: P_LATE_CULL,
        between: "slot 5's bracket, holding the EARLY raster scope (`vb.rs:2110-2309`); the armed \
                  legs additionally record the pyramid build at `vb.rs:2334`",
        margin_on_every_leg: true,
    },
    BeginPair {
        earlier: P_LATE_CULL,
        later: P_LATE_RASTER,
        between: "slot 7's bracket, holding the `if occlusion_split` late cull \
                  (`vb.rs:2505-2591`), plus the EMPTY `e7 -> b8` gap (`vb.rs:2592-2593`). The late \
                  cull is skipped on the two DISARMED legs, which therefore have no margin here",
        margin_on_every_leg: false,
    },
];

/// Clauses 3 and 4 for one worker: the leg-independent run's order, and slot 6's placement.
///
/// **`BOTTOM`-vs-`BOTTOM` only.** Slot 2's begin is `TOP_OF_PIPE` and is never compared here; it is
/// printed as an OBSERVATION instead.
fn assert_record_order(row: &BenchSummary, slot: Slot, round: usize) {
    let who = format!("`{}` round {round}", slot.label());
    let off = |p: usize| row.begin_off_ns[p];

    // ---- clause 3: the span that is identical on all four legs ---------------------------------
    //
    // The relation is `<=` on every row of the audit, including the rows that DO have a margin: a
    // margin makes equality surprising, never illegal, and asserting strictness where the plan
    // asked for order would add a failure mode nothing in the derivation supports.
    for pair in &BEGIN_CHAIN {
        assert!(
            off(pair.earlier) <= off(pair.later),
            "{who}: begin offsets are not monotone across the leg-independent run -- `{}` begins at \
             {:.1} ns and `{}` at {:.1}.\n\
             Between them: {}.\n\
             {}\n\
             ⚠️ This clause licenses ONE conclusion: a stamp is not at `BOTTOM_OF_PIPE`, or not at \
             the recorder site this harness assumes. It licenses NOTHING about the host's record \
             ORDER -- these are device completion times, and record order needs a host-side witness \
             (rung P4-3's `cull_uniform_filled` pattern, `vb.rs:1613`/`:1621`).",
            PASS_LABELS[pair.earlier],
            off(pair.earlier),
            PASS_LABELS[pair.later],
            off(pair.later),
            pair.between,
            if pair.margin_on_every_leg {
                "The relation asserted is `<=`, not `<`: prefix-completion times are non-decreasing \
                 in record position, and although this pair does have GPU work between the stamps \
                 on every leg, equality is a legal reading and not a defect."
            } else {
                "The relation asserted is `<=`, and strictness is NOT available here: two \
                 BOTTOM_OF_PIPE stamps with no command between them wait on prefixes differing by \
                 nothing, so they are only guaranteed to differ by the counter's lattice quantum \
                 (measured 0 on this machine, legally non-zero elsewhere). Do not `fix` this back \
                 to `<` -- rung P4-6's first sitting did exactly that and reported EQUAL offsets on \
                 both fixtures."
            }
        );
    }
    // Both operands are `end_off_ns`, each the median of PER-FRAME `(end − base)` values. Nothing
    // is composed from two reduced medians here, which is what rung P4-6 changed in the runner
    // after this clause reported the inequality backwards by 144 ns on a 47 us run.
    let end = |p: usize| row.end_off_ns[p];
    assert!(
        end(P_LATE_RASTER) <= end(P_RUN),
        "{who}: `vb_late_raster` ends at {:.1} ns and `vb_run` at {:.1}, so the late raster closes \
         AFTER the run that contains it.\n\
         Between `e8` and `e9` the recorder puts NOTHING (`vb.rs:2822-2823`, adjacent lines), so \
         the true per-frame margin is the counter's lattice quantum and the relation asserted is \
         `<=`. Both operands are `end_off_ns` -- medians of per-frame `(end - base)` -- so the \
         median-composition artifact that used to explain a red here is GONE, not tolerated.\n\
         ⚠️ Licensed conclusion: `e8` and `e9` are not the two stamps this harness thinks they are \
         -- one is at a different pipeline stage, or slot 8's end no longer immediately precedes \
         slot 9's. NOT licensed: anything about the host's record ORDER.",
        end(P_LATE_RASTER),
        end(P_RUN)
    );

    // ---- clause 4: slot 6's placement, which carries the whole "it left the run" claim ---------
    if slot.leg().hzb_inside_run() {
        assert!(
            end(P_EARLY_RASTER) <= off(P_HZB_BUILD),
            "{who}: `vb_hzb_build` begins at {:.1} ns, before `vb_early_raster` ends at {:.1}.\n\
             Between `e5` and `b6` the recorder puts a host-side probe counter and NO GPU command \
             (`vb.rs:2309-2334`), so the true per-frame margin is the counter's lattice quantum and \
             the relation asserted is `<=`. The left operand is `end_off_ns` -- a median of \
             per-frame `(end - base)` -- so the median-composition artifact that used to explain a \
             red here is GONE, not tolerated.\n\
             ⚠️ Licensed conclusion: slot 6's begin is not at the `vb.rs:2334` call site on this \
             leg, or slot 5's end is not at `vb.rs:2309`. NOT licensed: anything about the host's \
             record ORDER.",
            off(P_HZB_BUILD),
            end(P_EARLY_RASTER)
        );
        // Audit: between `b6` and `b7` the recorder puts slot 6's whole bracket -- the poison clear
        // plus the per-mip build dispatches inside `record_hzb_poison_build` (`vb.rs:4882`) -- and
        // the `e6 -> b7` gap, which holds only the EARLY-DEPTH dump copy (`vb.rs:2365`, gated
        // `occlusion_split && scene.hzb_dump`, which no timing leg arms). A margin therefore exists
        // on every armed leg; `<=` is asserted anyway, because a margin makes equality surprising
        // and not illegal.
        assert!(
            off(P_HZB_BUILD) <= off(P_LATE_CULL),
            "{who}: `vb_hzb_build` begins at {:.1} ns, after `vb_late_cull` at {:.1}.\n\
             Between them: slot 6's bracket (the poison clear + the per-mip build dispatches, \
             `vb.rs:4882`) and the `e6 -> b7` gap, which holds only the EARLY-DEPTH dump copy that \
             no timing leg arms.\n\
             ⚠️ Licensed conclusion: one of the two stamps is not where this harness places it. NOT \
             licensed: anything about the host's record ORDER -- these are device completion times \
             (rung P4-3's host-side witness is the pattern for an order claim).",
            off(P_HZB_BUILD),
            off(P_LATE_CULL)
        );
    } else {
        // Audit: between `e9` (`vb.rs:2823`) and the DISARMED call site (`vb.rs:3416`'s
        // `if !occlusion_split`) the recorder puts the whole lit-producer dispatch -- the
        // classified (`vb.rs:3127`) or fused (`vb.rs:3294`) arm. This is the one pair in the set
        // whose margin is a large, unconditional block of GPU work, so STRICTNESS IS AVAILABLE here
        // and is kept: the lattice quantum bounds a pair with nothing between them, and this pair
        // has a shade dispatch between them.
        assert!(
            off(P_HZB_BUILD) > end(P_RUN),
            "{who}: `vb_hzb_build` begins at {:.1} ns, INSIDE the run (which ends at {:.1}). On a \
             disarmed leg `record_hzb_poison_build` is called from its OTHER site (`vb.rs:3416`), \
             after the lit producer and therefore after `vb_run`'s end stamp -- and this clause \
             carries the whole 'slot 6 left the run' claim, which is why `PlumbRun` may subtract \
             `m_6(B)` at all.\n\
             The margin here is the WHOLE shade dispatch, and the right operand is `end_off_ns` \
             (reduced whole), so neither the counter's lattice quantum nor a median composition can \
             produce this reading.\n\
             ⚠️ It is a statement about completion TIMES -- that slot 6's interval lies outside the \
             run's -- and not about the host's record sequence, which timestamps cannot witness.",
            off(P_HZB_BUILD),
            end(P_RUN)
        );
    }
}

// ===============================================================================================
// The report
// ===============================================================================================

/// The scope paragraph and **THE PREDICTION**, printed BEFORE the first worker spawns.
///
/// A prediction written after the numbers is not a prediction. This one is printed at the head of
/// the run so the run can contradict it, and [`print_prediction_outcome`] says whether it did.
fn print_preamble(fixture: Fixture, k: u32, rounds: usize, long_frames: u32, bench_frames: u32) {
    println!(
        "=== VG R3 P4-6 -- the measurement, fixture `{}`{} ===",
        fixture.name(),
        if fixture == Fixture::Dense { format!(" (K={k})") } else { String::new() }
    );
    println!(
        "  protocol: {rounds} rounds of A0 -> B -> C -> A1; channel W two-point subtracted over \
         N2={long_frames} minus N1={SHORT_FRAMES} frames; channel G over {bench_frames} timed \
         frames per worker past the runner's own warm-up."
    );
    println!(
        "  SCOPE, before any number: every leg is a FULLY SERIALIZED frame (`wait_idle` per bench \
         frame, on top of unconditional FIFO) -- correct for timestamp deltas, NOT the frame the \
         shipped renderer executes. `Bracketed` is not end-to-end and this repository still has no \
         end-to-end number: channel W, the only end-to-end channel, is KNOWN-BLIND. One machine, \
         one driver, one sitting. `Saving` under MOTION is not measured and cannot be (D12: the \
         pyramid is a fixed point on a static scene). The marker's own gather cost is on no leg. \
         The cull-readback probe's cost is on no leg (the runner refuses it with the bench at boot)."
    );
    println!(
        "  ⚠️ NetRun is a paired difference of two structurally identical intervals; its residual \
         bias is second-order and unsigned, and NO directional confidence is claimed for either \
         sign of the result."
    );
    println!("  ---- THE PREDICTION, WRITTEN BEFORE THIS RUN ----");
    match fixture {
        Fixture::Mixed => println!(
            "  On `vb_occ_mixed`: `Saving` will NOT clear its band -- 8 low-poly instances at 512^2, \
             where the early raster is dominated by scope fixed cost -- while `Overhead` will be a \
             measurable magnitude. A run contradicting either half is a finding about the \
             INSTRUMENT and is reported as one."
        ),
        Fixture::Dense => println!(
            "  On `vb_occ_dense` at K={k}: `Saving` SHOULD clear its band if the instrument \
             resolves at all. A run contradicting that is a finding about the INSTRUMENT and is \
             reported as one."
        ),
    }
    println!("  -------------------------------------------------");
}

/// Channel W: the reading, the wreck claim, and the present-limit reading taken rather than assumed.
fn print_channel_w(fixture: Fixture, wall: &[Vec<f64>; 4], long_frames: u32, rounds: usize) {
    println!(
        "CHANNEL W (us/frame) -- KNOWN-BLIND. Two-point subtracted over N2={long_frames} minus \
         N1={SHORT_FRAMES}, {rounds} rounds, fixture `{}`. It decides NOTHING; it answers one \
         question: did arming the instrument WRECK the frame?",
        fixture.name()
    );
    for slot in SLOTS {
        let v = &wall[slot.idx()];
        println!(
            "  {:<20} median={:>10.1}  CV={:>6.2}%  n={}",
            slot.label(),
            median(v),
            100.0 * cv(v),
            v.len()
        );
    }
    let base = median(&wall[Slot::A0.idx()]);
    let period_ms = base / 1000.0;
    println!(
        "  PRESENT LIMIT (measured, not assumed): baseline period {period_ms:.3} ms/frame ({:.1} \
         Hz). The swapchain is VK_PRESENT_MODE_FIFO_KHR unconditionally, so this channel is bounded \
         BELOW by the display refresh -- which is why it is KNOWN-BLIND rather than merely noisy.",
        1000.0 / period_ms.max(f64::MIN_POSITIVE)
    );
    for slot in [Slot::B, Slot::C] {
        let d = relative(&wall[Slot::A0.idx()], &wall[slot.idx()]);
        let verdict = if d > WRECK_THRESHOLD {
            "WRECKED (>20% frame-period inflation over A0)"
        } else {
            "inside my own noise"
        };
        println!("  {:<20} vs A0 = {:+.2}%   [{verdict}]", slot.label(), 100.0 * d);
    }
}

/// Channel G: the ten passes, four legs, medians over rounds of each worker's own median.
fn print_channel_g(fixture: Fixture, gpu: &[Vec<BenchSummary>; 4], rounds: usize) {
    println!(
        "CHANNEL G (ns) -- the ten VB zone brackets on `{}`, {rounds} rounds. Each cell is the \
         median over rounds of that worker's median over its timed frames.",
        fixture.name()
    );
    println!(
        "  {:<16} {:>12} {:>12} {:>12} {:>12} {:>12} {:>12}",
        "pass", "A0", "B", "C", "A1", "D(C-B)", "D(A1-A0)"
    );
    for (p, label) in PASS_LABELS.iter().enumerate() {
        let m = |i: usize| median(&gpu[i].iter().map(|s| s.median_ns[p]).collect::<Vec<_>>());
        let (a0, b, c, a1) = (m(0), m(1), m(2), m(3));
        println!(
            "  {label:<16} {a0:>12.1} {b:>12.1} {c:>12.1} {a1:>12.1} {:>+12.1} {:>+12.1}",
            c - b,
            a1 - a0
        );
    }
    println!(
        "  ⚠️ Per-slot numbers are NOT exclusive costs: within-run migration between slots 3..8 is \
         zero-sum by the BOTTOM_OF_PIPE partition property but redistributes freely. ⚠️ `vb_hzb_build` \
         is not comparable across an armed/disarmed pair -- its recorder site MOVES. ⚠️ `vb_shade`'s \
         begin is TOP_OF_PIPE (kept for VB-P1d compatibility), so its OFFSET orders nothing."
    );
}

/// The published quantities, their bands, and the magnitudes for which no band is claimed.
fn print_quantities(
    banded: &[&Quantity; 6],
    overhead: &[f64],
    net: &[f64],
    plumb: &[f64],
    late_share: &[f64],
    d4: &Quantity,
    quantum: f64,
) {
    println!(
        "THE QUANTITIES (ns; positive = MORE time). band = max(FLOOR, TWIN), both printed:\n\
         \x20 FLOOR = this reading's own RESOLUTION -- the propagated standard error of every \
         median it is built from (`SE ~ 1.2533*sigma/sqrt(n)` with `sigma ~ (p95-median)/1.645`), \
         sub-floored per median at the measured lattice quantum {quantum:.1} ns.\n\
         \x20 TWIN  = DRIFT, the identical reduction run on the A1-vs-A0 zero control interleaved \
         inside the same rounds: max(|median Q0|, p90|Q0|).\n\
         \x20 ⚠️ The TWIN alone was the plan's whole band (§C2) and it is structurally ZERO here: A0 \
         and A1 are the same configuration on a serialized deterministic GPU, so 'clears the noise' \
         degenerates to 'is nonzero'. A band that is all-FLOOR and a band that is all-TWIN mean \
         different things -- read both columns."
    );
    for q in banded {
        println!(
            "  {:<14} {:>+12.1}   band={:>10.1} (floor={:>9.1} twin={:>9.1})   [{}]   + => {}",
            q.name,
            q.value(),
            q.band(),
            q.floor_term(),
            q.twin_term(),
            q.verdict(),
            q.positive_means
        );
    }
    println!(
        "  ---- reported as MAGNITUDES WITH A SCALE, never as significance verdicts ----\n\
         \x20 No band is claimed for these. A0 and A1 are both disarmed, so slots 3/7/8 bracket \
         empty blocks on both and their zero twin is the lattice quantisation: testing 'the late \
         passes cost more than nothing' against it is unfalsifiable AND trivially true."
    );
    println!("  {:<14} {:>+12.1}   (attribution only, no bound in either direction)", "Overhead", median(overhead));
    println!("  {:<14} {:>+12.1}   (the per-slot attribution sum over all six in-run slots)", "Net", median(net));
    println!(
        "  {:<14} {:>+12.1}   (attribution-grade: removing an interval from inside a partition \
         cannot undo migration into or out of it)",
        "PlumbRun",
        median(plumb)
    );
    println!(
        "  {:<14} {:>12.3}   (Overhead / m_5(A0) -- the machinery's cost as a fraction of the \
         un-split early raster it exists to shrink)",
        "LateShare",
        median(late_share)
    );
    println!(
        "  {:<14} {:>+12.1}   band={:>10.1} (floor={:>9.1} twin={:>9.1})   (SCALE, not a \
         significance verdict: the occlusion LEAF's own bracket -- one lane per batch, serial inner \
         loop)",
        d4.name,
        d4.value(),
        d4.band(),
        d4.floor_term(),
        d4.twin_term()
    );
}

/// The two TOP-vs-BOTTOM comparisons, printed and deciding nothing.
fn print_observations(gpu: &[Vec<BenchSummary>; 4]) {
    println!(
        "OBSERVATIONS (TOP vs BOTTOM -- not ordered by record position). `vb_shade`'s begin is a \
         TOP_OF_PIPE write, which waits only for prior commands to REACH the pipe top, so a \
         later-recorded TOP stamp may legally report an EARLIER time than a BOTTOM one. These \
         decide nothing and are printed so the fact is on the record rather than in a comment."
    );
    for slot in SLOTS {
        let rows = &gpu[slot.idx()];
        let shade = median(&rows.iter().map(|s| s.begin_off_ns[P_VB_SHADE]).collect::<Vec<_>>());
        // `end_off_ns`, not `begin_off_ns + median_ns`: an observation printed from a composed
        // quantity is as wrong as an assertion made from one, and quieter about it.
        let run_end = median(&rows.iter().map(|s| s.end_off_ns[P_RUN]).collect::<Vec<_>>());
        println!(
            "  {:<20} off(vb_shade)={shade:>12.1}   end(vb_run)={run_end:>12.1}   [{}]",
            slot.label(),
            if shade > run_end { "shade begins after the run ends" } else { "shade begins before the run ends" }
        );
        if !slot.leg().hzb_inside_run() {
            let hzb = median(&rows.iter().map(|s| s.begin_off_ns[P_HZB_BUILD]).collect::<Vec<_>>());
            println!(
                "  {:<20} off(vb_hzb_build)={hzb:>12.1}   vs off(vb_shade)={shade:>12.1}   \
                 [OBSERVATION only -- the disarmed leg's slot 6 sits past the shade producer]",
                ""
            );
        }
    }
}

/// The decision table, with the row this reading selects marked.
fn print_decision(
    net_run: &Quantity,
    saving: &Quantity,
    brack: &Quantity,
    late_share: &[f64],
    d4: &Quantity,
) {
    let (nr, nb) = (net_run.value(), net_run.band());
    let (sv, sb) = (saving.value(), saving.band());
    let (bk, bb) = (brack.value(), brack.band());
    let row = if nr < -nb && bk > bb {
        "THE DECISION PAYS, THE PLUMBING EATS IT -- the target is `PlumbRun` (late upload / second \
         scope), an R4+ rung, not a default change."
    } else if nr < -nb {
        "THE DECISION PAYS -- publish; the field's doc may recommend `TwoPhase` for occluder-dense \
         scenes. THE DEFAULT STILL DOES NOT MOVE (see below)."
    } else if nr > nb && sv.abs() <= sb {
        "THE SPLIT COSTS MORE THAN IT SAVES -- the campaign's recorded finding stands ('the \
         bottleneck is GRANULARITY, not the test'). Default `Off` becomes a recommendation; the \
         next investment is the meshlet rung."
    } else if nr.abs() <= nb && bk.abs() <= bb {
        "NOT RESOLVED -- and that is a RESULT: the instrument resolves per-pass costs, these \
         fixtures do not separate the arms."
    } else {
        // ⚠️ This branch's stated reason was WRONG until rung P4-6's third sitting. It said
        // "NetRun and Bracketed disagree about direction", which is not what puts a reading here:
        // sitting 3 landed in this branch with BOTH positive (NetRun +9216, Bracketed +20480). The
        // real cause is the one below, and the refusal itself is unchanged -- no row is invented.
        "NO SINGLE ROW. The reading satisfies the COST row's NetRun half (`NetRun > +band`) but \
         violates its other half (`|Saving| <= band`): this fixture shows a net COST and a \
         measurable SAVING at the same time, and the decision table has no row for that pair. \
         Reported as-is -- inventing a row after the fact is how a reading gets fitted to a \
         conclusion."
    };
    println!("DECISION TABLE ROW: {row}");
    // The rung's actual RESULT, stated plainly and independently of which row was selected: the
    // signs of the two cost-side quantities. Both positive means the split costs more than it saves
    // on this fixture, whatever the table does with the pair.
    if nr > nb && bk > bb {
        println!(
            "  ⚠️ THE RESULT, PLAINLY: NetRun = {nr:+.1} ns and Bracketed = {bk:+.1} ns are BOTH \
             positive and both clear their bands. On this fixture THE SPLIT COSTS MORE THAN IT \
             SAVES -- the run bracket is longer with the decision armed, and so is the sum of the \
             bracketed ranges against the disarmed baseline. `Saving` = {sv:+.1} ns is real and is \
             smaller than what the machinery costs to obtain it."
        );
    }
    if median(late_share) > 1.0 {
        println!(
            "  ALSO: LateShare = {:.3} > 1.0 -- the machinery costs MORE than the raster it \
             shrinks. Descriptive, no band.",
            median(late_share)
        );
    }
    if d4.value().abs() > d4.band() {
        println!(
            "  ALSO: D_4 = {:+.1} ns clears its own band ({:.1}) -- THE OCCLUSION LEAF IS THE FIRST \
             THING TO CHANGE if the split is pursued: one lane per batch, serial inner loop.",
            d4.value(),
            d4.band()
        );
    }
    println!(
        "  ⚠️ WHAT WOULD JUSTIFY DEFAULT-ON, and why this rung cannot supply it: `NetRun < -band` \
         across >=3 sittings on >=2 fixtures of different occlusion density, AND a second consumer \
         for the pyramid so `HzbBuild` is not charged to this feature alone. P4-6 is ONE campaign, \
         two fixtures, one machine, one sitting, no second consumer. THE DEFAULT STAYS `Off`, and \
         that is a scope statement rather than an inconclusive measurement."
    );
}

/// Whether the prediction printed before the run survived it.
fn print_prediction_outcome(fixture: Fixture, saving: &Quantity, overhead: &[f64]) {
    let resolved = saving.value().abs() > saving.band();
    let oh = median(overhead);
    match fixture {
        Fixture::Mixed => {
            println!(
                "PREDICTION vs OUTCOME (`mixed`): predicted `Saving` NOT resolved -- observed {}. \
                 Predicted `Overhead` a measurable magnitude -- observed {oh:+.1} ns.",
                if resolved { "RESOLVED (the prediction is CONTRADICTED)" } else { "NOT RESOLVED (as predicted)" }
            );
            if resolved {
                println!(
                    "  ⚠️ CONTRADICTED: a saving that clears its band on EIGHT low-poly instances at \
                     512^2 is a finding about the INSTRUMENT, not about the split. Before publishing \
                     it, re-check the non-vacuity clause and the zero control."
                );
            }
        }
        Fixture::Dense => {
            println!(
                "PREDICTION vs OUTCOME (`dense`): predicted `Saving` RESOLVED if the instrument \
                 resolves at all -- observed {}.",
                if resolved { "RESOLVED (as predicted)" } else { "NOT RESOLVED (the prediction is CONTRADICTED)" }
            );
            if !resolved {
                println!(
                    "  ⚠️ CONTRADICTED: with the hidden set replicated and the non-vacuity clause \
                     green, a `Saving` inside its own band says the early raster's cost is NOT \
                     dominated by the instances the split removes. That is a finding about this \
                     fixture's shape (the replicas share four screen neighbourhoods and z-reject \
                     each other) as much as about the split."
                );
            }
        }
    }
    println!("=== No threshold is pinned and no perf property is asserted. ===");
}

// ===============================================================================================
// `vb_occ_dense`'s correctness oracle
// ===============================================================================================

/// The engine frame index a capture must be at least at: the pyramid's boot clear makes frame 1
/// defer provably nothing, and the fixed point holds from frame 2.
const MIN_CONVERGED_FRAME: u32 = 3;

/// The replication factors the oracle gate runs.
const ORACLE_K: [u32; 3] = [1, 8, 64];

/// The part of a `BOYKO_HZB_DUMP` file this gate reads.
///
/// A LOCAL decoder rather than a borrowed one, for the reason `vb_occ_mixed.rs` gives about its own:
/// a gate that borrows another gate's parser inherits that gate's future edits.
struct PyramidDump {
    source: [u32; 2],
    levels: u32,
    flags: u32,
    frame_index: u32,
    /// Every mip, finest first, back to back, row-major, as `f32` — what `occlusion_verdict` folds.
    pyramid: Vec<f32>,
}

/// Little-endian `u32` at word index `i`.
fn word(bytes: &[u8], i: usize) -> u32 {
    let o = i * 4;
    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
}

fn decode_pyramid(bytes: &[u8], path: &Path) -> PyramidDump {
    assert!(
        bytes.len() >= HZB_DUMP_HEADER_BYTES as usize,
        "{}: {} bytes is shorter than the {}-byte header",
        path.display(),
        bytes.len(),
        HZB_DUMP_HEADER_BYTES
    );
    let magic = word(bytes, 0);
    assert_eq!(
        magic,
        HZB_DUMP_MAGIC,
        "{}: leading word is 0x{magic:08x}, not HZB_DUMP_MAGIC. A stale file decoded as this run's \
         evidence is what the driver's `remove_file` exists to prevent.",
        path.display()
    );
    let source = [word(bytes, 1), word(bytes, 2)];
    let levels = word(bytes, 3);
    let flags = word(bytes, HZB_DUMP_WORD_FLAGS);
    let frame_index = word(bytes, HZB_DUMP_WORD_FRAME_INDEX);

    let mut pyramid_texels = 0usize;
    for k in 0..levels as usize {
        let w0 = HZB_DUMP_HEADER_SCALAR_WORDS + 2 * k;
        pyramid_texels += word(bytes, w0) as usize * word(bytes, w0 + 1) as usize;
    }
    let depth_texels = source[0] as usize * source[1] as usize;
    let want = HZB_DUMP_HEADER_BYTES as usize
        + (2 * depth_texels + pyramid_texels) * HZB_DUMP_SAMPLE_BYTES as usize;
    assert_eq!(
        bytes.len(),
        want,
        "{}: {} bytes, but the header describes {want}. The file and its own header disagree.",
        path.display(),
        bytes.len()
    );

    let pyramid_word0 = HZB_DUMP_HEADER_BYTES as usize / 4 + 2 * depth_texels;
    let pyramid = (0..pyramid_texels).map(|i| f32::from_bits(word(bytes, pyramid_word0 + i))).collect();
    PyramidDump { source, levels, flags, frame_index, pyramid }
}

/// The raw right-hand side of `table.key` in the record probe's flat TOML subset.
fn probe_field(src: &str, path: &str, file: &Path) -> String {
    let (table, key) = path.split_once('.').expect("a probe path is `table.key`");
    let mut inside = false;
    for line in src.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.trim_start_matches('[').trim_end_matches(']') == table;
            continue;
        }
        if inside
            && let Some((kk, v)) = l.split_once('=')
            && kk.trim() == key
        {
            return v.trim().to_string();
        }
    }
    panic!("the record probe {} has no `{path}`", file.display())
}

fn probe_u32(src: &str, path: &str, file: &Path) -> u32 {
    probe_field(src, path, file).parse().unwrap_or_else(|_| panic!("`{path}` is not an integer"))
}

fn probe_bool(src: &str, path: &str, file: &Path) -> bool {
    match probe_field(src, path, file).as_str() {
        "true" => true,
        "false" => false,
        other => panic!("`{path}` is `{other}`, which is not a boolean"),
    }
}

/// Everything ONE dense capture worker produced.
struct DenseCapture {
    probe: CullProbe,
    dump: PyramidDump,
    draw_batches: u32,
    occlusion_instances: u32,
    scopes: u32,
}

/// Runs one dense worker at replication factor `k` with the three capture knobs armed.
fn run_dense_capture(k: u32) -> DenseCapture {
    let cull_out: PathBuf = std::env::temp_dir().join(format!("boyko_vb_occ_dense_{k}_cull.txt"));
    let dump_out: PathBuf = std::env::temp_dir().join(format!("boyko_vb_occ_dense_{k}_hzb.bin"));
    let probe_out: PathBuf = std::env::temp_dir().join(format!("boyko_vb_occ_dense_{k}_probe.toml"));
    // A stale file this run failed to overwrite would be read as this run's evidence, and "the
    // capture never ran" and "the capture left last run's file" are the same bytes.
    for p in [&cull_out, &dump_out, &probe_out] {
        let _ = std::fs::remove_file(p);
    }

    let mut cmd = base_worker_cmd(Fixture::Dense, Leg::Armed, k);
    // ⚠️ All three are armed in ONE process: the cull's verdicts and the pyramid they were tested
    // against are one frame's evidence only if one frame produced both.
    cmd.args(["--nocapture"])
        .env("BOYKO_VB_CULL_READBACK", &cull_out)
        .env("BOYKO_HZB_DUMP", &dump_out)
        .env("BOYKO_VB_PROBE", &probe_out);
    let status = cmd.status().expect("invariant: the dense capture worker spawns");
    assert!(status.success(), "the dense capture worker (K={k}) exited {status}");

    let cull_text = std::fs::read_to_string(&cull_out).unwrap_or_else(|e| {
        panic!(
            "K={k}: the CULL capture wrote no line at {} ({e}). A worker that renders and produces \
             nothing is an instrument failure, not an empty scene.",
            cull_out.display()
        )
    });
    let dump_bytes = std::fs::read(&dump_out).unwrap_or_else(|e| {
        panic!("K={k}: the PYRAMID capture wrote no file at {} ({e})", dump_out.display())
    });
    let probe_text = std::fs::read_to_string(&probe_out).unwrap_or_else(|e| {
        panic!("K={k}: the RECORD probe wrote no file at {} ({e})", probe_out.display())
    });

    assert!(
        probe_bool(&probe_text, "host.vb_path", &probe_out)
            && probe_bool(&probe_text, "host.mesh_leg", &probe_out),
        "K={k}: the probed frame is not a `VisibilityBuffer x Mesh` frame, so its counts say \
         nothing about the cull. This is an instrument failure, not a gate result."
    );
    DenseCapture {
        probe: parse_probe_line(cull_text.trim()),
        dump: decode_pyramid(&dump_bytes, &dump_out),
        draw_batches: probe_u32(&probe_text, "host.draw_batches", &probe_out),
        occlusion_instances: probe_u32(&probe_text, "host.occlusion_instances", &probe_out),
        scopes: probe_u32(&probe_text, "probe.scopes", &probe_out),
    }
}

/// Which of the two admissible ring layouts the engine produced, identified from whether batch-local
/// offset `0` is a candidate.
///
/// Each mesh has exactly ONE unmarked instance, spawned FIRST, and the six-plus marked ones share an
/// archetype. The ECS gather scatters with a per-mesh cursor over the query's iteration order, so
/// the only free variable is WHICH archetype the query yields first:
///
/// * `false` — the UNMARKED archetype first: ring `[U, M0, M1, …]`, so slot `s` is spawn `s` and
///   offset 0 is the unmarked instance, which is never a candidate;
/// * `true` — the MARKED archetype first: ring `[M0, M1, …, U]`, so slot `s` is spawn `s + 1` and
///   offset 0 is `M0`, a HIDDEN instance the oracle rejects — hence always a candidate.
///
/// The two are therefore DISTINGUISHABLE by one bit, and the identification is derived rather than
/// predicted: a predicted layout would make a kernel iteration-order change read as a cull defect.
fn dense_ring_marked_first(cap: &DenseCapture) -> bool {
    let mut chosen: Option<bool> = None;
    for b in 0..vb_occ_dense::BATCH_COUNT {
        let (base, members) = &cap.probe.late_cand[b];
        // An EMPTY candidate list identifies "unmarked first" by accident, and the set-equality
        // clause below would then red naming the ring rather than the empty deferral. Refuse here.
        assert!(
            !members.is_empty(),
            "RING LAYOUT: batch {b} deferred NOTHING, so the layout is unidentifiable. Every batch \
             of this fixture carries 2K hidden instances behind the slab; an empty candidate list \
             is either a fixture that stopped occluding or a cull that stopped deciding, and \
             neither is a ring-order question."
        );
        let marked_first = members.iter().any(|g| g == base);
        match chosen {
            None => chosen = Some(marked_first),
            Some(prev) => assert_eq!(
                prev, marked_first,
                "RING LAYOUT: batch 0 identified marked_first={prev} and batch {b} \
                 marked_first={marked_first}. One ECS archetype iteration order serves the whole \
                 world, so the two batches cannot differ."
            ),
        }
    }
    chosen.expect("invariant: BATCH_COUNT >= 1")
}

/// **THE DENSE FIXTURE'S CORRECTNESS ORACLE** — the GPU's deferral set against
/// [`boyko_render::hzb::occlusion_verdict`] computed on the host, at `K ∈ {1, 8, 64}`.
///
/// # Why this and not an A/B hash
///
/// On a converged static scene `ARMED == FORCE_KEEP` pixels is plan D12 RESTATED, and on a fixture
/// with no pixel pin that is worth nothing. The oracle is independent of the engine's decision: it
/// folds the DUMPED pyramid with the engine's own `select_texels`/`occluder_depth` and compares the
/// verdict set against the candidate lists the GPU actually produced.
///
/// # What it still cannot claim
///
/// Nothing pins this fixture's PIXELS. A defect that produces the oracle's verdicts and the wrong
/// image is invisible here; pixel correctness stays `[vb_occ_mixed]`'s job, on 8 instances.
#[test]
#[ignore = "live GPU gate (spawns three windowed capture workers); run with --test-threads=1"]
fn vb_occ_dense_defers_what_the_host_oracle_rejects() {
    for k in ORACLE_K {
        vb_occ_dense::assert_fixture_invariants(k);
        let cap = run_dense_capture(k);
        let instances = vb_occ_dense::dense_instances(k);

        // ---- the instrument: the frame converged, split, and carried the whole scene ------------
        assert!(
            cap.dump.frame_index >= MIN_CONVERGED_FRAME && cap.probe.frame == cap.dump.frame_index,
            "K={k}: the capture is at engine frame {} (dump {}), not a converged frame >= \
             {MIN_CONVERGED_FRAME} from ONE frame. The pyramid's boot clear makes frame 1 defer \
             provably nothing, so an unconverged capture would report `S n_defer == 0` correctly \
             and prove nothing.",
            cap.probe.frame,
            cap.dump.frame_index
        );
        assert_eq!(
            cap.probe.gpu_frame, cap.probe.frame,
            "K={k}: the cull read frame {} out of `VbCullUniform` while the host was on {}",
            cap.probe.gpu_frame, cap.probe.frame
        );
        assert_eq!(
            cap.occlusion_instances as usize,
            vb_occ_dense::marked_total(k),
            "K={k}: {} of the {} instances carried `OcclusionCulling` into the ring",
            cap.occlusion_instances,
            vb_occ_dense::marked_total(k)
        );
        assert_eq!(cap.draw_batches as usize, vb_occ_dense::BATCH_COUNT, "K={k}: draw batches");
        assert_eq!(cap.scopes, 2, "K={k}: the recorder reported {} raster scopes", cap.scopes);
        assert_ne!(
            cap.dump.flags & HZB_DUMP_FLAG_DEPTH_EARLY,
            0,
            "K={k}: the dump header's HZB_DUMP_FLAG_DEPTH_EARLY is clear, so this frame did not \
             split and every clause below would adjudicate the wrong pyramid."
        );
        assert_eq!(
            cap.dump.source,
            [vb_occ_mixed_scene::EXTENT, vb_occ_mixed_scene::EXTENT],
            "K={k}: the dump was taken at {:?}, not the extent this fixture's pixel arithmetic is \
             stated against",
            cap.dump.source
        );
        let layout = HzbLayout::new(cap.dump.source[0], cap.dump.source[1])
            .expect("invariant: the engine built a pyramid over this extent");
        assert_eq!(cap.dump.levels, layout.levels(), "K={k}: dump levels vs the oracle's layout");

        // Every per-batch lane must carry one entry per drawn batch BEFORE anything indexes them:
        // a short lane would panic with a message naming this reader instead of the emitter.
        for (name, len) in [
            ("late_cnt_pre", cap.probe.late_cnt_pre.len()),
            ("late_cand", cap.probe.late_cand.len()),
        ] {
            assert_eq!(
                len,
                vb_occ_dense::BATCH_COUNT,
                "K={k}: the probe's `{name}=` lane carries {len} entries for {} drawn batches",
                vb_occ_dense::BATCH_COUNT
            );
        }

        // ---- the oracle's verdict for every MARKED instance -------------------------------------
        let marked_first = dense_ring_marked_first(&cap);
        let mut total_defer = 0usize;
        for b in 0..vb_occ_dense::BATCH_COUNT {
            let mesh = vb_occ_dense::mesh_of_batch(b);
            let of_mesh = vb_occ_dense::indices_of_mesh(&instances, mesh);
            let n = of_mesh.len();
            let (base, members) = &cap.probe.late_cand[b];

            // What the GPU deferred, as spawn positions within this mesh.
            let mut gpu_deferred: Vec<usize> = members
                .iter()
                .map(|g| {
                    let slot = (g - base) as usize;
                    assert!(
                        slot < n,
                        "K={k} batch {b}: candidate global {g} is at ring slot {slot} of a \
                         {n}-instance batch based at {base}"
                    );
                    if marked_first { (slot + 1) % n } else { slot }
                })
                .collect();
            gpu_deferred.sort_unstable();

            // What the ORACLE rejects, over the same mesh's marked instances.
            let mut oracle_rejected: Vec<usize> = (0..n)
                .filter(|&s| {
                    let inst = &instances[of_mesh[s]];
                    inst.role.is_marked() && dense_verdict(&cap.dump, &layout, inst) == OcclusionVerdict::Reject
                })
                .collect();
            oracle_rejected.sort_unstable();

            assert_eq!(
                cap.probe.late_cnt_pre[b] as usize,
                members.len(),
                "K={k} batch {b}: `late_cnt_pre` is {} but the candidate region holds {}",
                cap.probe.late_cnt_pre[b],
                members.len()
            );
            assert_eq!(
                gpu_deferred,
                oracle_rejected,
                "K={k} batch {b} ({mesh:?}, ring marked_first={marked_first}): the GPU deferred the \
                 spawn positions {gpu_deferred:?} while the host oracle rejects {oracle_rejected:?} \
                 over the DUMPED pyramid. These are the same predicate over the same bytes on a \
                 converged frame, so a disagreement is the cull's arithmetic, the fixture's \
                 placement, or the ring identification -- and the fixture's placement is asserted \
                 without a GPU by `vb_occ_dense::assert_fixture_invariants`, which ran first."
            );
            total_defer += members.len();
        }

        // The scaling claim, stated as its own clause so a K-independent constant cannot satisfy it.
        assert_eq!(
            total_defer,
            vb_occ_dense::hidden_total(k),
            "K={k}: `S n_defer` is {total_defer}, not the {} hidden instances. The whole point of \
             this fixture is that the deferral count SCALES with K; a count that does not is either \
             a fixture that stopped occluding or a cull that stopped deciding.",
            vb_occ_dense::hidden_total(k)
        );
        println!(
            "VG R3 P4-6 `vb_occ_dense` K={k}: frame={} batches={} S n_defer={total_defer} \
             (= 4K, oracle-agreed on every instance)",
            cap.probe.frame, cap.draw_batches
        );
    }
}

/// The oracle's verdict for one dense instance against the DUMPED pyramid.
fn dense_verdict(dump: &PyramidDump, layout: &HzbLayout, inst: &DenseInstance) -> OcclusionVerdict {
    let (mn, mx) = vb_occ_dense::instance_world_aabb(inst);
    occlusion_verdict(layout, &dump.pyramid, &vb_occ_mixed_scene::view_proj_rows(), mn, mx)
}

// ===============================================================================================
// The fixture's own arithmetic, without a GPU
// ===============================================================================================

/// `vb_occ_dense`'s invariants at every `K` the oracle gate uses, plus the default.
///
/// Runs in a plain `cargo test`: a "tidying" edit to the replication lattice reds here rather than
/// arriving at the next GPU sitting as a count nobody can attribute.
#[test]
fn the_dense_fixture_is_internally_consistent() {
    for k in ORACLE_K.into_iter().chain([vb_occ_dense::DEFAULT_K]) {
        vb_occ_dense::assert_fixture_invariants(k);
    }
    // The two roles the protocol's counts rest on, at the default K.
    let instances = vb_occ_dense::dense_instances(vb_occ_dense::DEFAULT_K);
    assert_eq!(
        instances.iter().filter(|i| i.role == Role::Hidden).count(),
        vb_occ_dense::hidden_total(vb_occ_dense::DEFAULT_K)
    );
    assert_eq!(instances.iter().filter(|i| i.role == Role::Visible).count(), 2);
    for mesh in [MixedMesh::Sphere, MixedMesh::Cube] {
        assert!(!vb_occ_dense::marked_indices_of_mesh(&instances, mesh).is_empty());
    }
}
