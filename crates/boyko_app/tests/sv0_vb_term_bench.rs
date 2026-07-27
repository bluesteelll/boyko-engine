//! **VB-SV0 rung S5 — measure** (`docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 8, §6 "S5 — measure"): a
//! COUNTERBALANCED (ABBA) interleaved paired A/B of the **VB lit-producer dispatch**, SV0 armed
//! (`sv0_mode = 3`) against `sv0_mode = 0`, on S1's `vb_both_sdf` fixture at 512², over the two
//! structurally different tails — matrix **row 1** (`vb_resolve`, fused) and **row 7**
//! (`vb_shade_split`).
//!
//! This is the last rung, and its output feeds exactly one decision: plan §7 clause 3's abort.
//!
//! # What differs from S1.5, and why it is not a detail
//!
//! S1.5 ([`sv0_deferred_term_bench`](../sv0_deferred_term_bench.rs)) A/B'd a **push constant** —
//! `FineMarcherPush::lighting_flags`, four bytes in the recorded stream, one pipeline, one
//! descriptor set, one command-stream shape. SV0's own gate is **light-header word 7 bits 5..6**
//! (plan §3.1), a word in a buffer the shader reads. So this rung's phase travels
//! `LightingConfig` → `sync_sv0_light_gate` → `collect_lights` → `LightTableGeneration` →
//! `light_upload_due` → the fenced slot's staging copy. Three consequences:
//!
//! 1. **The dispatch is still byte-identical between phases.** Same `.spv`, same pipeline, same
//!    sets, same push, same group count; the tail reads one wave-uniform header word
//!    (`vb_resolve.comp.hlsl:376`) and branches. That word IS the shipped gate, so this measures
//!    the feature rather than a proxy for it.
//! 2. **The per-frame light upload is uniform past the warm-up, and that is load-bearing.** The
//!    phase moves at cycle positions 0→1 and 2→3 — twice per quadruple — and
//!    `FRAMES_IN_FLIGHT == 2` makes each generation bump cost two per-slot catch-ups, so every
//!    frame past the first uploads. The 20-frame warm-up is discarded, so no kept sample straddles
//!    the transient. This is not merely tidy: the copy itself sits outside the bracket, but the
//!    reader-side `TRANSFER→COMPUTE` barrier the framegraph derives for it is recorded at the
//!    lit-producer pass, INSIDE it. An upload cadence that differed between phases would put a
//!    barrier in one arm and not the other — a difference in the measured span, not around it.
//! 3. **The CPU re-pack is NOT uniform, and the counterbalance is what removes it.**
//!    `collect_lights` rebuilds only on positions 1 (CLEARED) and 3 (ARMED). That contamination
//!    `c` enters `d1 = m0 − m1` as `−c` and `d2 = m3 − m2` as `+c`, so `(d1 + d2)/2` cancels it
//!    exactly and `(d1 − d2)/2` reports it. A strict ABAB here would have carried `c` in every
//!    delta with a fixed sign — the S1.5 failure, in a second costume.
//!
//! # The protocol is S1.5's, unchanged — and S1.5 is why
//!
//! Read `sv0_deferred_term_bench.rs`'s module doc for the derivations; they are not repeated here.
//! The three things it learned the hard way all apply, and the runner block implements them by
//! calling the SAME helpers (`sv0_quadruple_stats`, `sv0_median_ns`, `sv0_p10_p90_ns`,
//! `sv0_tick_evidence`, `sv0_half_split_medians`), not by re-deriving them:
//!
//! * **ABBA, not ABAB.** `FRAMES_IN_FLIGHT == 2` makes a strict alternation alias the A/B phase
//!   perfectly against the frame-in-flight slot. S1.5's first harness reported −2048 ns on two
//!   IDENTICAL configurations because of exactly that.
//! * **A null control with a threshold fixed before the run** ([`SV0_S5_NULL_CONTROL_MAX_FRACTION`]),
//!   able to fail.
//! * **The quantisation floor is MEASURED, not assumed** — and it is a BOUND, not an equality.
//!   S1.5 reported `tick_gcd = 1024` while `timestampPeriod` reports 1, and read that as the
//!   counter's step. ⚠️ **This rung's own eight sessions falsified that**: seven reported 1024, one
//!   reported **128**, so the step is at most 128 ns and 1024 was an artifact of sample
//!   homogeneity (a GCD over observed durations is a MULTIPLE of the step whenever the durations
//!   cluster). See `sv0_deferred_term_bench.rs`'s CORRECTION section. The consequence for the
//!   protocol is [`SV0_LATTICE_MIN_DISTINCT_TICKS`]: the lattice may widen the spread gate only
//!   when its bound rests on enough distinct tick values, and
//!   [`sv0_s5_instrument_resolves_its_signal`] still asserts SEPARATELY — and non-waivably — that
//!   the lattice term does not bind.
//!
//! ≥ [`SV0_S5_BENCH_MIN_QUADS`] quadruples, warm-up discarded, median paired delta as the
//! statistic, [`SV0_S5_BENCH_SESSIONS`] separate processes per row, cross-session spread reported
//! and gated.
//!
//! # ⚠️ Warm-up is discarded at TWO levels, because the first session of a process set is cold
//!
//! The first eight-session run produced a `row1_armed1` median of 549376 ns against 12800 / 13312
//! for the two sessions after it — 42x, on a protocol whose gate is 10%. The 20-frame IN-SESSION
//! warm-up ([`SV0_S5_BENCH_MIN_QUADS`]'s sibling, `boyko_app`'s `SV0_BENCH_WARMUP`) did not touch
//! it, because sessions 2 and 3 ran that same warm-up and were fine. The cold start is at PROCESS
//! level.
//!
//! So `scripts\sv0_s5_bench.ps1` runs a **discarded warm-up SESSION** first, per row, before the
//! three that are transcribed as medians. Three things make that a design rather than a widened
//! threshold:
//!
//! 1. **It is discarded, not down-weighted.** A robust statistic over more sessions would ABSORB
//!    the cold session — and this whole finding exists only because the harness refused to absorb a
//!    disagreement it could not explain. Absorbing is the failure mode, not the fix.
//! 2. **It stays VISIBLE.** Its median is transcribed (`SV0_S5_<R>_WARMUP_DELTA_NS`) and
//!    [`sv0_s5_warmup_session_is_disclosed`] prints its ratio to the kept sessions' central value.
//!    A cold session that is merely dropped leaves no way to tell a harness that needed the drop
//!    from one that did not.
//! 3. **Its ticks still count as evidence about the INSTRUMENT.** A cold session's durations are
//!    integer tick counts on the same counter whatever the clocks were doing, so its
//!    `quantum_max_ns` pools with the rest — and in the observed run it was the cold session, with
//!    its far wider range of durations, that supplied the 128 ns bound. Discarded as a measurement
//!    of the TERM; kept as a measurement of the INSTRUMENT.
//!
//! A longer in-session warm-up was NOT chosen: no evidence points at an in-session ramp (sessions
//! 2 and 3 ran the same 20 frames and agreed), and it would cost every session time to fix
//! something that is not there. It remains the contingent remedy, and the summary line's
//! `median_delta_first_half_ns` / `median_delta_second_half_ns` are what would select it — halves
//! that disagree mean a session was still settling while it recorded.
//!
//! # ⚠️ The adjudication, and the two corrections it carries
//!
//! §7 clause 3 aborts if S5's median paired delta exceeds **2×** `SV0_DEFERRED_TERM_REFERENCE`,
//! which S1.5 measured at 6144 ns. That reference must be corrected TWICE before the comparison,
//! and both corrections are in the false-GREEN direction if skipped.
//!
//! ## Correction 1 — deflate the reference (S1.5's own finding)
//!
//! `pc.lighting_flags` gates TWO arms of `sdf_gbuffer_composite.hlsl`: the `!own_pixel` arm SV0
//! mirrors (`:1865`) and the `own_pixel` SDF-hit arm (`:1805`). S1.5's delta covers both, so it
//! OVER-states the mirrored term. `sv0_s1_5_confound_set_is_bounded` measures the confound set on
//! this fixture; [`sv0_s5_confound_deflation_is_derived_not_assumed`] re-derives the covered-pixel
//! denominator here from the shipped oracle and computes the deflation
//! `1 / (1 + sdf_visible / covered)` in code rather than copying a factor.
//!
//! **An inflated reference RAISES the abort threshold**, i.e. it lets a more expensive SV0 ship.
//! That is why the correction is computed and gated rather than mentioned.
//!
//! It is an **upper bound on the deflation factor**, not an estimate: it assumes the two arms cost
//! the same per pixel, and the `own_pixel` arm — which marches from an SDF surface hit — plausibly
//! costs more. If it does, the true `!own_pixel` share is smaller, the deflation stronger, and the
//! threshold lower still. The bound therefore errs toward permitting, which is stated here so a
//! borderline PASS is read for what it is.
//!
//! ## Correction 2 — the comparison is per-FRAME, and S4's percentages are NOT a denominator
//!
//! The plan does not say whether to compare per-frame or per-armed-pixel. **Per-frame nanoseconds,
//! on the same fixture at the same extent**, is the right reading, and the alternative rests on a
//! misreading of S4:
//!
//! * §7 clause 3's own rationale is *"a ratio to a measured sibling that already ships this visual
//!   at an accepted cost"*. Two implementations of one visual on one scene: the commensurable
//!   quantity is what each costs to deliver that scene, i.e. wall-clock nanoseconds per frame.
//! * S4 gate (ii) measured **10.18–10.28%** (shadow) and **3.07–3.31%** (contact AO) of 28362
//!   covered mesh pixels. Those are **CHANGED-pixel fractions** — `S4_MIN_CHANGED_FRACTION` /
//!   `S4_MAX_CHANGED_FRACTION` in `sv0_arm_matrix.rs` name them exactly that. They are NOT the
//!   count of pixels that RAN the term. The shipped SV0 blocks are gated on the mode bits and
//!   `NoL > SHADOW_NDOTL_EPS`, not on being shadowed: `ao_final = min(ao_final, sdf_ao(P, n))`
//!   and the `sdf_soft_shadow_ranged` call execute on essentially every covered mesh pixel, and
//!   only ~10% / ~3% of them end up visibly different. Dividing a cost by a changed-pixel count
//!   would normalise by a number with no relation to the work performed.
//! * Both sides already run over the same set. The Deferred `!own_pixel` arm covers the
//!   raster-owned pixels; SV0 covers the covered mesh pixels; S1's oracle asserts
//!   `MeshSelection::sdf_occluded == 0` on this fixture, i.e. the two sets are EQUAL here. A
//!   per-pixel normalisation would divide both sides by the same 28362 and change nothing but the
//!   units — unless it used the changed-pixel count, which would change the verdict for the wrong
//!   reason.
//!
//! So: per-frame ns, both numbers on `vb_both_sdf` at 512². The S4 percentages are REPORTED as
//! context (evidence the term does visible work at this cost) and read by no gate.
//!
//! # ⚠️ §7 clause 5 is a DEFINED OUTCOME, not a dead end
//!
//! If the cross-session spread exceeds its gate, or the lattice binds, this rung has produced a
//! legitimate result: *the instrument cannot decide the number at this scale*. Clause 5 makes that
//! an owner VALUES call — revert, or ship unmeasured with the spread recorded and clause 3
//! explicitly waived — because clause 3 divides two numbers an irreproducible instrument makes
//! incommensurable. Every failure message below says so, and none of them is licence to widen a
//! threshold.
//!
//! # Env knobs
//!
//! - `BOYKO_SV0_S5_BENCH=1` (any value) — arms the shared `record_vb` timestamp collector and the
//!   runner's ABBA A/B loop. Unset ⇒ this test behaves exactly like an ordinary windowed run: no
//!   collector, no reset/write commands, `LightingConfig` untouched by the runner — a
//!   byte-identical command stream and no golden moves.
//! - `BOYKO_SV0_S5_BENCH_QUADS=<n>` — the TIMED quadruple budget (default 200, `boyko_app::runner`).
//! - `BOYKO_SV0_S5_BENCH_NULL=1` (any value) — **the null control.** Both phases request the ARMED
//!   mode, so the two configurations are IDENTICAL and the reported median is pure residual.
//! - `BOYKO_SV0_SSAO=1` — selects **row 7** (`vb_shade_split`); unset selects **row 1**
//!   (`vb_resolve`). The SAME knob `sv0_scene` already ships for the S4 matrix.
//! - `BOYKO_SV0_MODE` / `BOYKO_SV0_FROXEL` must be UNSET. The runner drives the SV0 request every
//!   frame (so a stray `BOYKO_SV0_MODE` is overwritten, not honoured), but `BOYKO_SV0_FROXEL`
//!   would silently select rows 2/5/6 instead of the two the plan names.
//!
//! # Runbook — every session is a separate PROCESS
//!
//! ⚠️ **Dev profile, no `--release`** — the same profile S1.5's runbook used, so §7 clause 3
//! divides two numbers taken under the same host conditions. The `.spv` are identical either way
//! and both brackets are GPU wall-clock behind a per-frame `wait_idle`, so this is a
//! comparability disclosure rather than a claim of contamination; the summary line prints
//! `debug_assertions=` and [`sv0_s5_measurement_meets_its_gates`] pins it.
//!
//! ```text
//! $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-gnu'
//! powershell -File scripts\sv0_s5_bench.ps1            # 10 sessions -> D:\tmp\sv0_s5\*.log
//! ```
//!
//! or by hand, once per session:
//!
//! ```text
//! $env:BOYKO_DISABLE_VALIDATION='1'; $env:BOYKO_SV0_S5_BENCH='1'
//! Remove-Item Env:\BOYKO_SV0_S5_BENCH_NULL -ErrorAction SilentlyContinue
//! Remove-Item Env:\BOYKO_SV0_MODE          -ErrorAction SilentlyContinue
//! Remove-Item Env:\BOYKO_SV0_FROXEL        -ErrorAction SilentlyContinue
//! Remove-Item Env:\BOYKO_SV0_SSAO          -ErrorAction SilentlyContinue   # row 1; set '1' for row 7
//! cargo test -p boyko-app --test sv0_vb_term_bench sv0_vb_term_bench `
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Per row: ONE **discarded warm-up** session (identical command line to an armed one — it is
//! discarded by BOOKKEEPING, not by a different configuration), then THREE armed sessions, then
//! ONE null session (add `$env:BOYKO_SV0_S5_BENCH_NULL='1'`). **Ten sessions total.** Each prints
//! two transcribable lines plus, in this dev-profile build, the recorder's own row-identity line:
//!
//! ```text
//! boyko_rhi_vulkan: VB lit producer = <name>
//! VB-SV0-S5 mode=… row=… quads=… samples=… extent=…x… debug_assertions=…
//!           median_delta_ns=… median_order_bias_ns=… median_armed_ns=… median_cleared_ns=…
//!           p10_delta_ns=… p90_delta_ns=… p10_bias_ns=… p90_bias_ns=…
//!           median_delta_first_half_ns=… median_delta_second_half_ns=…
//! VB-SV0-S5 RESOLUTION: timestamp_period_ns=… tick_gcd=… distinct_ticks=…
//!           min_tick_gap=… tick_span=… quantum_max_ns=… median_lattice_max_ns=…
//!           timestamp_valid_bits=… timestamp_compute_and_graphics=…
//! ```
//!
//! ## The transcription table
//!
//! `<R>` is `ROW1` for the `BOYKO_SV0_SSAO`-unset sessions, `ROW7` for the set ones.
//!
//! | printed field | destination |
//! |---|---|
//! | armed `median_delta_ns` ×3 | `SV0_S5_<R>_SESSION_MEDIAN_DELTA_NS` |
//! | armed `quads` ×3 | `SV0_S5_<R>_SESSION_QUADS` |
//! | armed `median_order_bias_ns` ×3 | `SV0_S5_<R>_SESSION_ORDER_BIAS_NS` |
//! | armed `median_armed_ns` (session 1) | `SV0_S5_<R>_MEDIAN_DISPATCH_NS` |
//! | **warm-up** `median_delta_ns` | `SV0_S5_<R>_WARMUP_DELTA_NS` |
//! | null `median_delta_ns` | `SV0_S5_<R>_NULL_MEDIAN_DELTA_NS` |
//! | null `median_order_bias_ns` | `SV0_S5_<R>_NULL_ORDER_BIAS_NS` |
//! | `row=` | `SV0_S5_<R>_ROW_LABEL` |
//! | the last `VB lit producer = ` line | `SV0_S5_<R>_PRODUCER` |
//! | `debug_assertions=` | [`SV0_S5_DEBUG_ASSERTIONS`] |
//! | `timestamp_period_ns` | [`SV0_S5_TIMESTAMP_PERIOD_NS`] |
//! | `quantum_max_ns`, POOLED by GCD over ALL TEN sessions | [`SV0_S5_QUANTUM_MAX_NS`] |
//! | `median_lattice_max_ns`, recomputed from the pooled bound | [`SV0_S5_MEDIAN_LATTICE_MAX_NS`] |
//! | `distinct_ticks` of the session that supplied the pooled bound | [`SV0_S5_QUANTUM_DISTINCT_TICKS`] |
//!
//! The script prints this block ready to paste, pooling included; it never writes it.
//!
//! ⚠️ **The `RESOLUTION:` line states BOUNDS, so sessions that disagree are not contradicting each
//! other.** `quantum_max_ns` is `G · gcd(observed multipliers)` — a MULTIPLE of the device's step
//! whenever the sample is homogeneous. Ten sessions therefore yield ten upper bounds on ONE device
//! property, and the strongest statement they jointly support is their **GCD**: not a majority
//! vote, and not "pick one". The warm-up session's bound POOLS IN even though its median is
//! discarded — a cold session is invalid evidence about the TERM and valid evidence about the
//! INSTRUMENT — and in the first run it was the cold session that supplied the tightest bound.
//! `timestamp_period_ns`, `timestamp_valid_bits` and `timestamp_compute_and_graphics` ARE flat
//! device properties and must agree EXACTLY; a disagreement there is a real finding.
//!
//! Also CHECK, and report rather than transcribe: `extent` reads `512x512` on every session (an
//! OS-clamped window measures a different per-pixel workload); `samples` is close to `4 * quads`
//! (a large shortfall means the stream dropped frames); no `boyko_render: VB-SV0 was requested`
//! clamp line appears in an armed log (it would mean both phases ran unarmed, i.e. the "armed"
//! session was a second null control); and each session's `median_delta_first_half_ns` /
//! `median_delta_second_half_ns` agree with each other (halves that disagree mean that session was
//! still settling while it recorded, which the discarded warm-up SESSION cannot fix — the remedy
//! there is a longer in-session warm-up).
//!
//! Then run the CPU gates — including S1.5's, which is what keeps this rung's transcribed
//! reference and confound honest:
//!
//! ```text
//! cargo test -p boyko-app --test sv0_deferred_term_bench -- --nocapture
//! cargo test -p boyko-app --test sv0_vb_term_bench       -- --nocapture
//! ```
//!
//! Windowed-test conventions (mirrors `sv0_deferred_term_bench.rs`): `#[ignore]` (needs a real
//! windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::mesh::Vertex;
use boyko_render::{
    GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig,
};
use boyko_scene::ViewUniform;

mod sv0_oracle;
mod sv0_scene;

use sv0_oracle::{Coverage, OracleVertex};

// ===========================================================================================
// Pre-registered protocol constants — fixed BEFORE any measurement exists
// ===========================================================================================

/// The plan's pair floor (§6 S1.5, inherited verbatim by §6 S5's "same protocol as S1.5").
const SV0_S5_BENCH_MIN_PAIRS: usize = 30;

/// The floor this harness enforces, at the level of the STATISTIC's own sample size: ≥ 30
/// completed ABBA QUADRUPLES, i.e. 60 pairs. Strictly stronger than
/// [`SV0_S5_BENCH_MIN_PAIRS`] — see `sv0_deferred_term_bench.rs`'s twin for the derivation.
const SV0_S5_BENCH_MIN_QUADS: usize = SV0_S5_BENCH_MIN_PAIRS;

/// The plan's session count. Three separate PROCESSES per row, not three windows of one process:
/// the failure mode being guarded against is per-process GPU clock/power state.
const SV0_S5_BENCH_SESSIONS: usize = 3;

/// **The reproducibility gate** (§6 S5: "reproducible (spread ≤ 10%)"). `(max − min) / median`
/// over the session medians — peak-to-peak over the central value, the same shape the VB-P1d
/// record's "~21% run-to-run spread" quotes, so this gate is commensurable with the precedent
/// that motivated it.
///
/// The EFFECTIVE gate is `max(this, measured median lattice / |median|)`; see
/// [`sv0_s5_measurement_meets_its_gates`] for why a gate finer than the instrument's own
/// resolution is unreadable, and [`sv0_s5_instrument_resolves_its_signal`] for the separate,
/// non-waivable assertion that the lattice term does NOT bind.
const SV0_S5_SESSION_SPREAD_MAX: f64 = 0.10;

/// **The null control's pre-registered threshold** (§7 clause 5: "`|median paired delta|` on two
/// *identical* configurations must be ≤ a pre-registered fraction of the armed delta, not `~0`").
///
/// Registered at the same 10% as [`SV0_S5_SESSION_SPREAD_MAX`], and for the same reason: a drift
/// floor larger than the gate's own tolerance would make the gate unreadable. Fixed here, before
/// any run, so it cannot be widened to rescue a failing control.
const SV0_S5_NULL_CONTROL_MAX_FRACTION: f64 = 0.10;

/// **The abort ratio** — §7 clause 3: "S5's median paired delta exceeds **2×** S1.5's measured
/// `SV0_DEFERRED_TERM_REFERENCE` on the same fixture … In `[1×, 2×]` it ships with the number
/// recorded; above 2×, revert."
const SV0_S5_ABORT_RATIO: f64 = 2.0;

/// **The evidence floor a lattice BOUND must clear before it is allowed to widen the spread gate.**
///
/// The `RESOLUTION:` line's `quantum_max_ns` is `G · gcd(m_1 … m_n)` over the `n` DISTINCT observed
/// durations. Under the generic model — multipliers behaving like independent uniform integers —
/// `P(gcd(m) = 1) = 1/ζ(n)`, so the chance the bound OVERSTATES the step is `1 − 1/ζ(n)`: 1.70% at
/// `n = 6`, **0.83% at `n = 7`**, 0.42% at `n = 8`. Seven is the smallest `n` that puts the
/// overstatement risk under 1%, and that derivation — not any observed count — is where this number
/// comes from. It is the same constant, derived the same way, in
/// `sv0_deferred_term_bench.rs`; the two files cannot import each other.
///
/// Two honesty notes it must be read with. Clustered durations are NOT generic, so this is a floor
/// on the evidence and never a guarantee; and the bound divides `min_tick_gap` deterministically,
/// so a session whose distinct values sit far apart cannot produce a tight bound however many of
/// them there are. Both figures are on the `RESOLUTION:` line for that reason.
///
/// What clearing it licenses is narrow and one-directional: ONLY the widening of
/// [`SV0_S5_SESSION_SPREAD_MAX`] in [`sv0_s5_measurement_meets_its_gates`]. A bound that fails it
/// is still printed, still transcribed and still read by
/// [`sv0_s5_instrument_resolves_its_signal`] — it simply cannot make the gate more permissive,
/// which is the only direction a degenerate sample could ever flatter a result.
///
/// This constant exists because eight sessions of THIS rung produced the finding: seven reported a
/// 1024 ns bound from a fixed-workload dispatch whose durations clustered on a handful of
/// 1024-multiples, and one reported 128. Without an evidence floor, a session set that happened to
/// be homogeneous would have handed the gate an 8x-flattering lattice and nothing would have said
/// so.
const SV0_LATTICE_MIN_DISTINCT_TICKS: usize = 7;

// ===========================================================================================
// TRANSCRIBED INPUTS from rung S1.5 — the reference this rung is adjudicated against
// ===========================================================================================
//
// These are not S5 measurements. They are copies of numbers `sv0_deferred_term_bench.rs` owns,
// restated here because §7 clause 3's arithmetic needs them and the two test binaries cannot
// import each other. Each carries the live instrument that keeps it honest, and the runbook
// re-runs that instrument in the same breath as this one.

/// `SV0_DEFERRED_TERM_REFERENCE_NS` as `sv0_deferred_term_bench.rs` transcribed it — the Deferred
/// cost of the SDF soft-shadow + contact-AO term on this fixture at 512², the median of that
/// rung's three session medians.
///
/// Guarded there by `sv0_s1_5_measurement_meets_its_gates`, which asserts it IS that median. If
/// this literal and that one ever disagree the runbook's two CPU test runs disagree loudly.
const SV0_DEFERRED_TERM_REFERENCE_NS: f64 = 6144.0;

/// The confound set S1.5's `sv0_s1_5_confound_set_is_bounded` printed: pixels where the
/// `own_pixel` SDF-hit arm runs, i.e. the part of the reference SV0 does NOT mirror.
///
/// The one number here that is copied rather than re-derived. Re-deriving it needs the ray
/// generation and analytic sphere root that live in S1.5's file; a third copy of that geometry is
/// how two instruments silently drift, and the denominator it is divided by IS re-derived below
/// ([`sv0_s5_confound_deflation_is_derived_not_assumed`]), which catches the fixture moving under
/// either of them.
const SV0_S1_5_CONFOUND_SDF_VISIBLE_PX: usize = 3525;

/// The covered-mesh-pixel denominator the confound ratio is taken over — S1's raster at 512².
/// RE-DERIVED in this file from the shipped oracle, not trusted
/// ([`sv0_s5_confound_deflation_is_derived_not_assumed`]).
const SV0_SCENE_COVERED_MESH_PIXELS: usize = 28362;

/// S4 gate (ii)'s measured changed-pixel fractions, `[min, max]` over the eight armable rows —
/// shadow term. **Context only; no gate reads this.**
///
/// Recorded here because the obvious-looking use of it is wrong and the plan does not forbid it in
/// so many words: these are CHANGED pixels, not EXECUTED pixels, so they are not a per-pixel
/// denominator for a cost. See this file's module doc, "Correction 2".
const SV0_S4_SHADOW_CHANGED_FRACTION_RANGE: [f64; 2] = [0.1018, 0.1028];

/// S4 gate (ii)'s measured changed-pixel fractions, `[min, max]` — contact-AO term. Context only;
/// see [`SV0_S4_SHADOW_CHANGED_FRACTION_RANGE`].
const SV0_S4_AO_CHANGED_FRACTION_RANGE: [f64; 2] = [0.0307, 0.0331];

// ===========================================================================================
// MEASURED values — do not edit these literals to make a failing run pass
// ===========================================================================================
//
// Every literal in this block is a TRANSCRIPTION of a `VB-SV0-S5 …` line the harness printed.
// The standing discipline: a measured literal may be RE-measured, never adjusted. If a gate below
// fails, the finding is that the gate failed — the remedy is the fixture, the protocol or an
// abort under §7, never the number.
//
// `f64::NAN` / `""` / `None` are the UNMEASURED sentinels. None of them is a value that could be
// mistaken for evidence: every comparison against NaN is false, so a forgotten transcription
// cannot produce a passing gate — it produces the explicit "not measured" failure in `measured()`,
// which names the runbook.

/// **Row 1** (`vb_resolve`, fused) — the three sessions' `median_delta_ns`, in run order.
const SV0_S5_ROW1_SESSION_MEDIAN_DELTA_NS: [f64; SV0_S5_BENCH_SESSIONS] =
    [12800.0, 12800.0, 12800.0];
/// Row 1 — the three sessions' `quads`, same order (so the floor is quantified over the runs that
/// happened, not over the runner's default).
const SV0_S5_ROW1_SESSION_QUADS: [usize; SV0_S5_BENCH_SESSIONS] = [200, 200, 200];
/// Row 1 — the three sessions' `median_order_bias_ns`, the `(d1 − d2)/2` estimate of the ordering
/// + ring-slot + light-re-pack contamination the counterbalance cancels.
const SV0_S5_ROW1_SESSION_ORDER_BIAS_NS: [f64; SV0_S5_BENCH_SESSIONS] =
    [0.0, 0.0, 0.0];
/// Row 1 — session 1's `median_armed_ns`: the WHOLE lit-producer dispatch. Reported, never gated;
/// it is the scale statement that says how small the term is relative to the pass carrying it.
const SV0_S5_ROW1_MEDIAN_DISPATCH_NS: f64 = 41984.0;
/// Row 1 — the null control's `median_delta_ns` (two IDENTICAL configurations).
const SV0_S5_ROW1_NULL_MEDIAN_DELTA_NS: f64 = 0.0;
/// Row 1 — the null control's `median_order_bias_ns`: a FOURTH independent estimate of the same
/// physical quantity as [`SV0_S5_ROW1_SESSION_ORDER_BIAS_NS`], since the position effect does not
/// depend on the A/B word.
const SV0_S5_ROW1_NULL_ORDER_BIAS_NS: f64 = 0.0;
/// Row 1 — the `row=` label the summary printed (`fused` expected).
const SV0_S5_ROW1_ROW_LABEL: &str = "fused";
/// Row 1 — the LAST `boyko_rhi_vulkan: VB lit producer = ` name in the session log (`vb_resolve`
/// expected). The recorder derives it from the BOUND PIPELINE HANDLE, so it is an observation of
/// which `.spv` ran, not a restatement of the selector.
const SV0_S5_ROW1_PRODUCER: &str = "vb_resolve";
/// Row 1 — the DISCARDED warm-up session's `median_delta_ns`: the first process of this row's set,
/// which pays whatever the process-level cold start costs.
///
/// Transcribed but read by NO gate on the term. A cold session that is merely dropped leaves no way
/// to tell a harness that needed the drop from one that did not, so this is recorded and
/// [`sv0_s5_warmup_session_is_disclosed`] prints its ratio to the kept sessions' central value. A
/// value close to that central says the cold start was absent on this run; a value far from it says
/// the discarded session earned its place.
const SV0_S5_ROW1_WARMUP_DELTA_NS: f64 = 12800.0;

/// **Row 7** (`vb_shade_split`) — the three sessions' `median_delta_ns`, in run order.
const SV0_S5_ROW7_SESSION_MEDIAN_DELTA_NS: [f64; SV0_S5_BENCH_SESSIONS] =
    [14336.0, 14848.0, 10240.0];
/// Row 7 — the three sessions' `quads`.
const SV0_S5_ROW7_SESSION_QUADS: [usize; SV0_S5_BENCH_SESSIONS] = [200, 200, 200];
/// Row 7 — the three sessions' `median_order_bias_ns`.
const SV0_S5_ROW7_SESSION_ORDER_BIAS_NS: [f64; SV0_S5_BENCH_SESSIONS] =
    [0.0, -512.0, -512.0];
/// Row 7 — session 1's `median_armed_ns` (the whole split lit-producer dispatch).
const SV0_S5_ROW7_MEDIAN_DISPATCH_NS: f64 = 398848.0;
/// Row 7 — the null control's `median_delta_ns`.
const SV0_S5_ROW7_NULL_MEDIAN_DELTA_NS: f64 = 0.0;
/// Row 7 — the null control's `median_order_bias_ns`.
const SV0_S5_ROW7_NULL_ORDER_BIAS_NS: f64 = -512.0;
/// Row 7 — the `row=` label the summary printed (`split` expected).
const SV0_S5_ROW7_ROW_LABEL: &str = "split";
/// Row 7 — the LAST `VB lit producer = ` name in the session log (`vb_shade_split` expected).
const SV0_S5_ROW7_PRODUCER: &str = "vb_shade_split";
/// Row 7 — the DISCARDED warm-up session's `median_delta_ns` (see
/// [`SV0_S5_ROW1_WARMUP_DELTA_NS`]).
const SV0_S5_ROW7_WARMUP_DELTA_NS: f64 = 10240.0;

/// The `debug_assertions=` field, which must agree across all eight sessions and must be `true`
/// (the dev profile S1.5's runbook also used — see this file's runbook note on comparability).
///
/// `None` is the unmeasured sentinel: a `bool` has no NaN.
const SV0_S5_DEBUG_ASSERTIONS: Option<bool> = Some(true);

/// `VkPhysicalDeviceLimits::timestampPeriod` as the harness read it — ns per GPU timestamp TICK.
/// The SCALE, not the STEP.
const SV0_S5_TIMESTAMP_PERIOD_NS: f64 = 1.0;

/// The `quantum_max_ns` bound, **POOLED BY GCD over all ten sessions**: an UPPER BOUND on the
/// counter's step, in nanoseconds — `quantum <= this`, never `quantum == this`.
///
/// Each session's own figure is `G · gcd(observed multipliers)`, a MULTIPLE of the hardware step
/// `G` whenever that session's durations clustered. Ten sessions therefore give ten upper bounds on
/// ONE device property, and their GCD is the strongest statement they jointly support. Pooling is
/// not averaging and not voting: it is the meet of ten bounds, and it includes the discarded
/// warm-up sessions, whose ticks are valid evidence about the instrument even though their medians
/// are not valid evidence about the term.
///
/// No Vulkan limit reports the step. S1.5 read its own 1024 ns figure as a measurement of it; this
/// rung's first eight sessions falsified that (seven at 1024, one at 128) and
/// `sv0_deferred_term_bench.rs`'s CORRECTION section records the retraction.
const SV0_S5_QUANTUM_MAX_NS: f64 = 1024.0;

/// The `median_lattice_max_ns` bound recomputed from [`SV0_S5_QUANTUM_MAX_NS`]: an upper bound on
/// the lattice the REPORTED session median lands on — `quantum / 2` (each quadruple statistic is a
/// half-sum of two multiples of the quantum), halved again to `quantum / 4` when the quadruple
/// count is even.
///
/// Bounds the smallest non-zero difference two sessions' medians can show. A cross-session spread
/// at or below it would be quantisation rather than stability — but only as far as the bound is
/// tight, which is what [`SV0_S5_QUANTUM_DISTINCT_TICKS`] says.
const SV0_S5_MEDIAN_LATTICE_MAX_NS: f64 = 256.0;

/// How many DISTINCT tick values [`SV0_S5_QUANTUM_MAX_NS`] rests on — `distinct_ticks` from the
/// session that supplied the pooled bound.
///
/// This is the evidence figure, and it is what decides whether the lattice may WIDEN the spread
/// gate ([`SV0_LATTICE_MIN_DISTINCT_TICKS`]). `None` is the unmeasured sentinel and is treated
/// exactly like insufficient evidence: no widening. That is deliberate — a forgotten transcription
/// must not be able to hand the gate a flattering lattice, and the safe default costs at most a
/// stricter gate.
const SV0_S5_QUANTUM_DISTINCT_TICKS: Option<usize> = Some(100);

// ===========================================================================================
// The fixture
// ===========================================================================================

/// `vb_both_sdf.rs::setup` verbatim — the S1 scene through its ONE shared entry point.
///
/// Deliberately identical to the S1 fixture's setup rather than a variant of it: §7 clause 3
/// compares this rung's number against S1.5's *on the same fixture*, so anything this file
/// changed about the scene would make the two sides of that ratio incommensurable — and §6 S1.5's
/// own module doc records rejecting a scene change for exactly that reason.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let materials_row: [Option<u16>; sv0_scene::MESH_ROW_COUNT] = [
        None,
        Some(red.index() as u16),
        Some(green.index() as u16),
        Some(gold.index() as u16),
        Some(blue.index() as u16),
    ];

    sv0_scene::spawn_scene(&mut commands, sphere, &materials_row);
}

/// **The S5 counterbalanced (ABBA) A/B bench** (one session per process, one ROW per session).
///
/// Renders S1's fixture through `VisibilityBuffer × Both` — the path SV0 ships on, and the only
/// one whose recorder brackets the lit producer. `legs: Both` is REQUIRED, not incidental: SV0's
/// arming predicate is `sdf_leg && sdf_shadows_wanted && !hwrt_denoise_or_vis_on`, so under
/// `GeometryLegs::Mesh` `sync_sv0_light_gate` clamps BOTH phases to `sv0_mode = 0` and the "armed"
/// run would be a second null control. `boyko_app::runner`'s S5 block asserts `vb_sdf_mesh_armable`
/// at boot for exactly that reason.
///
/// The ROW is selected by `BOYKO_SV0_SSAO`: unset ⇒ the fused `vb_resolve` (matrix row 1); `1` ⇒
/// SSAO arms `mesh_geo_shade_split` and the split `vb_shade_split` displaces it (row 7). Those are
/// §6 S5's two required tails, and the knob is `sv0_scene`'s own — not a second one invented here.
///
/// The window extent is `sv0_scene::DUMP_EXTENT`, the CERTIFIED extent, deliberately not a knob.
///
/// `#[ignore]`: needs a real windowed GPU device. See this file's module doc for the runbook and
/// the transcription table; see [`sv0_s5_measurement_meets_its_gates`] for where the numbers land.
#[test]
#[ignore = "needs a real windowed GPU device; BOYKO_SV0_S5_BENCH=1 [BOYKO_SV0_S5_BENCH_NULL=1] \
            [BOYKO_SV0_SSAO=1 for row 7] BOYKO_DISABLE_VALIDATION=1 -- --ignored --nocapture \
            --test-threads=1; the orchestrator runs 3 armed + 1 null session PER ROW"]
fn sv0_vb_term_bench() {
    let mut app = App::new();
    let plugins = EnginePlugins::window(
        "boyko_engine vb-sv0 s5 vb lit-producer term bench",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    );
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Requested AFTER `add_plugins` so this override wins over `RenderPathPlugin`'s `Deferred`
    // default — `vb_both_sdf.rs`'s own post-plugins insert, verbatim.
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
    });
    // The SAME env-driven config the S1/S4 fixtures use. `BOYKO_SV0_MODE` is irrelevant here (the
    // runner overwrites the request every frame from the ABBA position) but `BOYKO_SV0_SSAO` is
    // the row selector and `clusters_enabled` must stay off, so this goes through one shared
    // constructor rather than a second literal that could drift from it.
    app.insert_resource(sv0_scene::lighting_config_from_env());
    if let Some(ssao) = sv0_scene::ssao_config_from_env() {
        app.insert_resource(ssao);
    }
    app.run();
}

// ===========================================================================================
// The transcribed measurement, handed to the gates as VALUES
// ===========================================================================================

/// One row's transcribed session set.
///
/// The indirection is the same one `sv0_deferred_term_bench.rs` explains and load-bearing for the
/// same two reasons: it gives the "nothing has been run yet" state ONE failure that names the
/// runbook, and it keeps the gates' assertions out of compile-time-constant shape (a `const`-folded
/// `assert!` on the UNMEASURED sentinel would fail the BUILD, turning "this rung has not run yet"
/// from a red test into a broken workspace).
struct Row {
    /// `"row 1 (vb_resolve, fused)"` — used in failure text so a message names which row failed.
    label: &'static str,
    /// The three session medians, in run order.
    medians: [f64; SV0_S5_BENCH_SESSIONS],
    /// The three sessions' quadruple counts.
    quads: [usize; SV0_S5_BENCH_SESSIONS],
    /// The three sessions' order-bias medians.
    biases: [f64; SV0_S5_BENCH_SESSIONS],
    /// The whole lit-producer dispatch's median (session 1) — reported, never gated.
    dispatch_ns: f64,
    /// The null control's median paired delta.
    null_delta: f64,
    /// The null control's order-bias median.
    null_bias: f64,
    /// The DISCARDED warm-up session's median paired delta — recorded so the cold start is visible
    /// rather than silently absorbed, and read by no gate on the term.
    warmup_delta: f64,
    /// The `row=` label the summary printed.
    row_label: &'static str,
    /// The producer name the recorder logged.
    producer: &'static str,
    /// The `row=` label this row MUST have printed.
    expect_row_label: &'static str,
    /// The producer name this row MUST have bound.
    expect_producer: &'static str,
    /// The MEDIAN of the three session medians — this row's central value.
    central_ns: f64,
}

/// The whole transcribed measurement.
struct Measured {
    /// `timestamp_period_ns` — ns per GPU timestamp tick (the SCALE).
    period_ns: f64,
    /// `quantum_max_ns` — an UPPER BOUND on the counter's step, in ns, pooled by GCD.
    quantum_max_ns: f64,
    /// `median_lattice_max_ns` — an upper bound on the lattice the reported session median lands
    /// on.
    lattice_max_ns: f64,
    /// How many distinct tick values the pooled bound rests on.
    lattice_distinct: Option<usize>,
    /// The build profile the sessions ran under.
    debug_assertions: bool,
    /// The two rows §6 S5 requires, in matrix order.
    rows: [Row; 2],
}

impl Measured {
    /// Whether the transcribed lattice BOUND is allowed to widen the spread gate.
    ///
    /// A GCD over a homogeneous sample is a multiple of the true step, and a coarser lattice
    /// widens `max(protocol, lattice_floor)`. That is the ONE direction in which a degenerate
    /// sample can flatter a result, so the widening is licensed by evidence rather than granted by
    /// default: the pooled bound must rest on at least [`SV0_LATTICE_MIN_DISTINCT_TICKS`] distinct
    /// tick values, and an unrecorded count (`None`) counts as no evidence at all.
    ///
    /// Refusing to widen can only make the gate STRICTER, so a false `false` here costs a re-run,
    /// never a wrong verdict.
    fn lattice_may_widen(&self) -> bool {
        self.lattice_distinct.is_some_and(|n| n >= SV0_LATTICE_MIN_DISTINCT_TICKS)
    }
}

/// The median of one row's three session medians.
fn central_of(medians: [f64; SV0_S5_BENCH_SESSIONS]) -> f64 {
    let mut sorted = medians;
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("invariant: transcribed medians are finite"));
    sorted[SV0_S5_BENCH_SESSIONS / 2]
}

/// Every transcribed literal is present; returns them for the gates to read.
///
/// # Panics
///
/// With the runbook, until the orchestrator has run the eight GPU sessions and transcribed their
/// output. That red state is the point: S5 is the rung §7 clause 3 is adjudicated on, so an un-run
/// S5 must not read as a green rung.
fn measured() -> Measured {
    let finite = |xs: [f64; SV0_S5_BENCH_SESSIONS]| xs.iter().all(|x| x.is_finite());
    let complete = finite(SV0_S5_ROW1_SESSION_MEDIAN_DELTA_NS)
        && finite(SV0_S5_ROW1_SESSION_ORDER_BIAS_NS)
        && finite(SV0_S5_ROW7_SESSION_MEDIAN_DELTA_NS)
        && finite(SV0_S5_ROW7_SESSION_ORDER_BIAS_NS)
        && SV0_S5_ROW1_MEDIAN_DISPATCH_NS.is_finite()
        && SV0_S5_ROW1_NULL_MEDIAN_DELTA_NS.is_finite()
        && SV0_S5_ROW1_NULL_ORDER_BIAS_NS.is_finite()
        && SV0_S5_ROW7_MEDIAN_DISPATCH_NS.is_finite()
        && SV0_S5_ROW7_NULL_MEDIAN_DELTA_NS.is_finite()
        && SV0_S5_ROW7_NULL_ORDER_BIAS_NS.is_finite()
        && SV0_S5_ROW1_WARMUP_DELTA_NS.is_finite()
        && SV0_S5_ROW7_WARMUP_DELTA_NS.is_finite()
        && SV0_S5_TIMESTAMP_PERIOD_NS.is_finite()
        && SV0_S5_QUANTUM_MAX_NS.is_finite()
        && SV0_S5_MEDIAN_LATTICE_MAX_NS.is_finite()
        // Required here, unlike S1.5's: the corrected harness ALWAYS prints `distinct_ticks`, so a
        // `None` in this rung means a transcription from a stale log rather than a session set that
        // predates the field.
        && SV0_S5_QUANTUM_DISTINCT_TICKS.is_some()
        && SV0_S5_DEBUG_ASSERTIONS.is_some()
        && !SV0_S5_ROW1_ROW_LABEL.is_empty()
        && !SV0_S5_ROW1_PRODUCER.is_empty()
        && !SV0_S5_ROW7_ROW_LABEL.is_empty()
        && !SV0_S5_ROW7_PRODUCER.is_empty();
    assert!(
        complete,
        "VB-SV0 S5 NOT MEASURED YET (expected until the orchestrator runs the GPU sessions).\n\
         Run, in TEN separate processes (1 DISCARDED warm-up + 3 armed + 1 null, per row):\n  \
           powershell -File scripts\\sv0_s5_bench.ps1\n\
         or by hand, per session (PowerShell):\n  \
           $env:BOYKO_DISABLE_VALIDATION='1'; $env:BOYKO_SV0_S5_BENCH='1'\n  \
           # row 1: BOYKO_SV0_SSAO unset.  row 7: $env:BOYKO_SV0_SSAO='1'\n  \
           # warm-up: same command line as an armed session; it is discarded by BOOKKEEPING\n  \
           # null:  add $env:BOYKO_SV0_S5_BENCH_NULL='1'\n  \
           cargo test -p boyko-app --test sv0_vb_term_bench sv0_vb_term_bench \
             -- --ignored --nocapture --test-threads=1\n\
         Each session prints a `VB-SV0-S5 mode=…` line and a `VB-SV0-S5 RESOLUTION:` line; this \
         file's module doc carries the field-to-constant transcription table. The RESOLUTION line \
         states BOUNDS: pool `quantum_max_ns` across ALL TEN sessions by GCD (the script does it), \
         and transcribe the `distinct_ticks` behind the pooled value — a bound with no recorded \
         evidence is refused as a gate-widener."
    );

    Measured {
        period_ns: SV0_S5_TIMESTAMP_PERIOD_NS,
        quantum_max_ns: SV0_S5_QUANTUM_MAX_NS,
        lattice_max_ns: SV0_S5_MEDIAN_LATTICE_MAX_NS,
        lattice_distinct: SV0_S5_QUANTUM_DISTINCT_TICKS,
        debug_assertions: SV0_S5_DEBUG_ASSERTIONS
            .expect("invariant: the completeness check above rejected the None sentinel"),
        rows: [
            Row {
                label: "row 1 (vb_resolve, fused)",
                medians: SV0_S5_ROW1_SESSION_MEDIAN_DELTA_NS,
                quads: SV0_S5_ROW1_SESSION_QUADS,
                biases: SV0_S5_ROW1_SESSION_ORDER_BIAS_NS,
                dispatch_ns: SV0_S5_ROW1_MEDIAN_DISPATCH_NS,
                null_delta: SV0_S5_ROW1_NULL_MEDIAN_DELTA_NS,
                null_bias: SV0_S5_ROW1_NULL_ORDER_BIAS_NS,
                warmup_delta: SV0_S5_ROW1_WARMUP_DELTA_NS,
                row_label: SV0_S5_ROW1_ROW_LABEL,
                producer: SV0_S5_ROW1_PRODUCER,
                expect_row_label: "fused",
                expect_producer: "vb_resolve",
                central_ns: central_of(SV0_S5_ROW1_SESSION_MEDIAN_DELTA_NS),
            },
            Row {
                label: "row 7 (vb_shade_split, split)",
                medians: SV0_S5_ROW7_SESSION_MEDIAN_DELTA_NS,
                quads: SV0_S5_ROW7_SESSION_QUADS,
                biases: SV0_S5_ROW7_SESSION_ORDER_BIAS_NS,
                dispatch_ns: SV0_S5_ROW7_MEDIAN_DISPATCH_NS,
                null_delta: SV0_S5_ROW7_NULL_MEDIAN_DELTA_NS,
                null_bias: SV0_S5_ROW7_NULL_ORDER_BIAS_NS,
                warmup_delta: SV0_S5_ROW7_WARMUP_DELTA_NS,
                row_label: SV0_S5_ROW7_ROW_LABEL,
                producer: SV0_S5_ROW7_PRODUCER,
                expect_row_label: "split",
                expect_producer: "vb_shade_split",
                central_ns: central_of(SV0_S5_ROW7_SESSION_MEDIAN_DELTA_NS),
            },
        ],
    }
}

// ===========================================================================================
// The gates
// ===========================================================================================

/// The pre-registered protocol constants are self-consistent — checked independently of any
/// measurement, so it stays a live assertion during the window where nothing has been measured.
#[test]
fn sv0_s5_protocol_constants_are_pre_registered() {
    assert_eq!(
        SV0_S5_BENCH_MIN_PAIRS, 30,
        "the plan's pair floor is 30 (§6 S1.5, inherited by §6 S5); lowering it changes the \
         protocol, not the code"
    );
    assert_eq!(
        SV0_S5_BENCH_SESSIONS, 3,
        "the plan's session count is 3 (§6 S5); the cross-session spread is what this rung gates"
    );
    // Compile-time, so a widened gate fails the BUILD rather than a test run someone can forget to
    // invoke. Const-eval panics carry no formatting, hence the static text.
    const {
        assert!(
            2 * SV0_S5_BENCH_MIN_QUADS >= SV0_S5_BENCH_MIN_PAIRS,
            "the quadruple floor must imply the plan's pair floor (one quadruple = two pairs)"
        );
    }
    const {
        assert!(
            SV0_S5_SESSION_SPREAD_MAX > 0.0 && SV0_S5_SESSION_SPREAD_MAX <= 0.10,
            "the spread gate is 10% (plan §6 S5); it may be TIGHTENED on new evidence, never \
             widened"
        );
    }
    const {
        assert!(
            SV0_S5_NULL_CONTROL_MAX_FRACTION > 0.0
                && SV0_S5_NULL_CONTROL_MAX_FRACTION <= SV0_S5_SESSION_SPREAD_MAX,
            "the null control's drift floor must not exceed the spread gate it is read against — \
             otherwise a 'green' cross-session spread could be entirely drift"
        );
    }
    const {
        assert!(
            SV0_S5_ABORT_RATIO == 2.0,
            "§7 clause 3's abort ratio is 2x; raising it is a plan change, not a code change"
        );
    }
}

/// **The S5 reproducibility gate.** Adjudicates the transcribed measurements against everything
/// §6 S5 and §7 clause 5 require, PER ROW: the sessions exist, each cleared the quadruple floor,
/// the cross-session spread is within the EFFECTIVE gate, and the null control is below its
/// pre-registered fraction of the armed median.
///
/// # This test is RED until the measurement exists, and that is the point
///
/// §7 clause 3 is adjudicated on S5's number, so an un-run S5 must not read as a green rung — the
/// same "reddens by default … it fails unless the rung does the work" discipline the plan applies
/// to its own S2 gate (g). The failure text names the exact commands and the exact literals, so
/// the red state is a runbook rather than a puzzle.
#[test]
fn sv0_s5_measurement_meets_its_gates() {
    let m = measured();

    // The RESOLUTION line must be internally consistent, or the lattice the spread gate is read
    // against is not the one the device reported. `quantum_max_ns` is an INTEGER number of ticks
    // (it is a GCD of tick counts, and a GCD of ten such GCDs is still one) scaled by the period,
    // and `median_lattice_max_ns` is `quantum_max_ns` over 2 or 4. Both are cheap, and both catch a
    // mis-transcription that would quietly move a gate.
    assert!(
        m.period_ns > 0.0 && m.quantum_max_ns > 0.0,
        "the transcribed timestamp period ({}) and quantum bound ({}) must both be positive — a \
         zero here would make the resolution floor vanish and the spread gate look trustworthy for \
         the wrong reason",
        m.period_ns,
        m.quantum_max_ns
    );
    let tick_gcd = m.quantum_max_ns / m.period_ns;
    assert!(
        (tick_gcd - tick_gcd.round()).abs() <= 1e-6 && tick_gcd.round() >= 1.0,
        "quantum_max_ns / timestamp_period_ns = {tick_gcd} is not a whole number of ticks; the \
         RESOLUTION line was mis-transcribed (the quantum bound IS a tick GCD times the period, \
         and pooling ten of them by GCD keeps it one)"
    );
    let lattice_ratio = m.quantum_max_ns / m.lattice_max_ns;
    assert!(
        (lattice_ratio - 2.0).abs() <= 1e-6 || (lattice_ratio - 4.0).abs() <= 1e-6,
        "median_lattice_max_ns must be quantum_max_ns / 2 (odd quadruple count) or / 4 (even), but \
         quantum/lattice = {lattice_ratio}; the RESOLUTION line was mis-transcribed, or the pooled \
         bound was combined with a per-session lattice instead of being re-divided"
    );

    // The profile disclosure. §7 clause 3 divides this rung's number by S1.5's, and S1.5's runbook
    // is a plain dev-profile `cargo test`. The `.spv` are identical in either profile and both
    // brackets are GPU wall-clock behind a per-frame `wait_idle`, so this is a comparability
    // requirement rather than a proof of contamination — which is exactly why it is a transcribed
    // fact and not an argument.
    assert!(
        m.debug_assertions,
        "the S5 sessions reported debug_assertions=false, i.e. they ran in a RELEASE profile while \
         SV0_DEFERRED_TERM_REFERENCE_NS was measured in the dev profile S1.5's runbook prescribes. \
         §7 clause 3 divides the two, so they must be taken under the same host conditions — re-run \
         the sessions with a plain `cargo test` (no --release), or re-measure S1.5 in release and \
         say so here"
    );

    for r in &m.rows {
        for (i, quads) in r.quads.iter().enumerate() {
            assert!(
                *quads >= SV0_S5_BENCH_MIN_QUADS,
                "{}: session {i} collected {quads} quadruples, below the protocol floor of {} \
                 (§6 S1.5's ≥{} pairs, applied to the statistic's own sample size) — re-run that \
                 session, do not lower the floor",
                r.label,
                SV0_S5_BENCH_MIN_QUADS,
                SV0_S5_BENCH_MIN_PAIRS
            );
        }

        let central = r.central_ns;
        assert!(
            central > 0.0,
            "{}: the armed median paired delta is {central} ns, i.e. arming SV0 did not cost \
             measurable time. Either the A/B never reached the shader (check the log for the \
             `boyko_render: VB-SV0 was requested` clamp line, which means BOTH phases rendered \
             unarmed and the 'armed' session was a second null control) or the instrument is \
             blind — a bench that cannot see the term it exists to measure cannot bound SV0's cost",
            r.label
        );

        // The EFFECTIVE spread gate: the COARSER of the pre-registered 10% and the measured
        // lattice, because below the lattice "spread" and "quantisation" are the same number. So
        // this can never be a silent widening, `sv0_s5_instrument_resolves_its_signal` asserts
        // separately that the lattice term does NOT bind; if it ever does, that test goes red and
        // names the fact, and this one does not paper over it.
        //
        // The lattice is a BOUND whose tightness is a property of the SAMPLE, so a homogeneous
        // session set could hand this `max()` a flattering number — the exact defect this rung's
        // own eight sessions surfaced. The widening is therefore licensed by EVIDENCE
        // (`Measured::lattice_may_widen`) rather than granted by default; without it the gate stays
        // at the protocol value, which can only ever be stricter.
        let mut sorted = r.medians;
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("invariant: transcribed medians are finite"));
        let spread = (sorted[SV0_S5_BENCH_SESSIONS - 1] - sorted[0]) / central;
        let lattice_floor = m.lattice_max_ns / central.abs();
        let may_widen = m.lattice_may_widen();
        let effective_max = if may_widen {
            SV0_S5_SESSION_SPREAD_MAX.max(lattice_floor)
        } else {
            SV0_S5_SESSION_SPREAD_MAX
        };
        let bound_by = if !may_widen {
            "the pre-registered 10% protocol gate (the lattice bound may NOT widen it: it rests on \
             fewer than the required distinct tick values)"
        } else if lattice_floor > SV0_S5_SESSION_SPREAD_MAX {
            "the instrument's measured quantisation lattice bound"
        } else {
            "the pre-registered 10% protocol gate"
        };
        println!(
            "VB-SV0-S5 gate [{}]: median={central:.1}ns dispatch={:.1}ns spread={spread:.4} \
             effective_max={effective_max:.4} (protocol={SV0_S5_SESSION_SPREAD_MAX}, \
             lattice_floor<={lattice_floor:.4}, lattice_evidence={:?}, may_widen={may_widen}) \
             bound by {bound_by}",
            r.label, r.dispatch_ns, m.lattice_distinct
        );
        assert!(
            spread <= effective_max,
            "S5 RED [{}]: cross-session spread {spread:.4} exceeds the effective gate \
             {effective_max:.4} (protocol {SV0_S5_SESSION_SPREAD_MAX}, measured lattice floor \
             <= {lattice_floor:.4}, allowed to widen the gate: {may_widen}) over medians {:?}. The \
             instrument is not trustworthy at this scale, so §7's cost clause cannot be adjudicated \
             on this row — this is §7 clause 5's DEFINED OUTCOME (an owner VALUES call: revert, or \
             ship unmeasured with the spread recorded and clause 3 explicitly waived), NOT a \
             licence to widen the gate. If may_widen is false, the lattice term was REFUSED because \
             its pooled bound rests on fewer than {} distinct tick values — re-run so the sessions \
             sample more widely, do not reason about what the lattice 'probably' is. And check the \
             warm-up disclosure first: a cold session that reached the transcribed triple would \
             blow this spread open on its own",
            r.label,
            r.medians,
            SV0_LATTICE_MIN_DISTINCT_TICKS
        );

        let null_budget = SV0_S5_NULL_CONTROL_MAX_FRACTION * central;
        assert!(
            r.null_delta.abs() <= null_budget,
            "S5 NULL CONTROL FAILED [{}]: two IDENTICAL configurations produced a median paired \
             delta of {} ns, above the pre-registered budget of {null_budget} ns ({} × {central}). \
             Under the counterbalanced design the constant ordering bias is cancelled by \
             construction, so a residual this large is NOT S1.5's Revision-1 failure repeating — \
             it is a SECOND-order position effect (or a drop pattern quietly correlated with the \
             cycle), and no number this harness produced means anything until it is explained. \
             Note this row's null is also a strictly QUIETER stream than its armed run (with the \
             request constant the light header never moves, so no per-frame re-pack or upload \
             happens at all), i.e. a LOWER bound on the armed residual — a failing null is \
             therefore worse news than it looks. Read median_order_bias_ns beside it: a large bias \
             with a small null is the design working; a large null is the design failing",
            r.label,
            r.null_delta,
            SV0_S5_NULL_CONTROL_MAX_FRACTION
        );
    }
}

/// **The resolution disclosure** — asserts that the measured quantisation lattice does NOT bind
/// the spread gate on either row, i.e. that this instrument can resolve the signal finer than the
/// protocol tolerance it is judged at.
///
/// # Why this is a separate, non-waivable test
///
/// [`sv0_s5_measurement_meets_its_gates`] compares each spread against
/// `max(protocol, lattice_floor)`, because a gate finer than the instrument's own resolution
/// cannot be read. That `max` is correct AND it is exactly the shape a failing run could hide
/// inside: raise the lattice and the gate widens itself. So the lattice term is asserted here, on
/// its own, where widening it is not an option.
///
/// A failure here is NOT a code defect and NOT a bug to fix by editing this file. It is the honest
/// statement "this instrument cannot resolve better than ±X% at this signal size", which is
/// precisely §7 clause 5's defined outcome. The remedies, in order of preference:
///
/// 1. More quadruples (`BOYKO_SV0_S5_BENCH_QUADS`). This does NOT move the lattice — a median of
///    lattice-valued samples is lattice-valued however many you take — so it helps only if the
///    failure is marginal and driven by an odd/even quadruple count. Cheap, so try it first.
/// 2. Raise the signal by changing the measured scene. This DOES move the ratio, and it costs the
///    rung its meaning: §7 clause 3's `2×` comparison is against S1.5's number ON THIS FIXTURE, so
///    S1.5 would have to be re-measured on the altered scene in the same breath. Only with the
///    owner's agreement, and only applied to both rungs together.
/// 3. Waive clause 3 under §7 clause 5, recording this test's numbers as the reason.
#[test]
fn sv0_s5_instrument_resolves_its_signal() {
    let m = measured();
    for r in &m.rows {
        let central = r.central_ns;
        assert!(
            central > 0.0,
            "{}: the armed median is {central} ns; resolution cannot be judged against a \
             non-positive signal (see sv0_s5_measurement_meets_its_gates for what that means)",
            r.label
        );
        let lattice_floor = m.lattice_max_ns / central;
        println!(
            "VB-SV0-S5 resolution [{}]: quantum<={} ns median_lattice<={} ns signal_ns={central} \
             lattice_floor<={lattice_floor:.4} vs protocol {SV0_S5_SESSION_SPREAD_MAX} \
             (evidence: {:?} distinct tick values)",
            r.label, m.quantum_max_ns, m.lattice_max_ns, m.lattice_distinct
        );
        assert!(
            lattice_floor <= SV0_S5_SESSION_SPREAD_MAX,
            "S5 RESOLUTION-BOUND [{}]: this instrument cannot resolve better than ±{:.1}% at this \
             signal size ({} ns lattice bound on a {central} ns term), which is coarser than the {} \
             protocol gate. The cross-session spread therefore cannot distinguish drift from \
             quantisation, and §7's cost clause cannot be adjudicated on it. This is §7 clause 5's \
             DEFINED OUTCOME — a legitimate result, not a failure to code around. See this test's \
             doc for the three remedies; NONE of them is editing SV0_S5_SESSION_SPREAD_MAX or \
             SV0_S5_MEDIAN_LATTICE_MAX_NS. Note the lattice is an UPPER bound, so a failure here \
             may also mean the pooled bound is merely LOOSE — read distinct_ticks and min_tick_gap \
             off the RESOLUTION lines to tell 'the instrument is blunt' from 'these sessions were \
             homogeneous'",
            r.label,
            lattice_floor * 100.0,
            m.lattice_max_ns,
            SV0_S5_SESSION_SPREAD_MAX
        );
    }
}

/// **The ordering bias, read rather than merely cancelled.**
///
/// The counterbalance removes a constant ordering/ring-slot bias by algebra — and here it also
/// removes the light-table re-pack, which lands on cycle positions 1 and 3 only. That algebra is
/// sound only if the contamination really is constant over a quadruple, an assumption a design
/// that averages it away would leave permanently unexamined. So the harness estimates it per
/// quadruple and reports the median, and this test reads the four independent estimates per row
/// (three armed sessions plus the null control, since the position effect does not depend on the
/// A/B word) against each other.
///
/// # What is asserted, and what is only reported
///
/// REPORTED: the bias magnitude as a fraction of the signal — the contamination a strict ABAB
/// would have carried in every delta, which is the number that justifies the design existing.
///
/// ASSERTED: sign agreement across the four runs, but ONLY when every estimate exceeds the
/// instrument's own lattice. Below the lattice a sign is rounding, not a measurement. Above it, a
/// sign flip means the "ordering bias" is not a stable property of the harness at all — the
/// cancellation stays harmless but the real limitation is variance, and a reader must be told
/// rather than reassured.
///
/// ⚠️ **S5's bias has a component S1.5's does not**, and a reader comparing the two rungs' numbers
/// should expect them to differ for that reason: the per-phase light re-pack is CPU work outside
/// the bracket, so it enters the bias through submission timing rather than through the dispatch.
#[test]
fn sv0_s5_order_bias_is_reported() {
    let m = measured();
    for r in &m.rows {
        let central = r.central_ns;
        let biases = [r.biases[0], r.biases[1], r.biases[2], r.null_bias];
        println!(
            "VB-SV0-S5 order bias [{}]: armed={:?} null={} signal_ns={central} lattice_ns<={}",
            r.label, r.biases, r.null_bias, m.lattice_max_ns
        );
        for (i, b) in biases.iter().enumerate() {
            println!(
                "  run {i}: bias {b} ns = {:.1}% of the signal — the amount a strict ABAB \
                 alternation would have ADDED to every one of its deltas",
                100.0 * b / central
            );
        }

        // ⚠️ The lattice is an UPPER bound, so this threshold is CONSERVATIVE in the direction of
        // not asserting: a loose bound can only suppress the assertion (calling a real sign
        // "rounding"), never fire it on noise. That weakens the test rather than falsifying it,
        // which is the acceptable direction — but it is a second reason the bound's evidence
        // matters, and why `Measured::lattice_may_widen` exists rather than blanket trust.
        let resolvable = biases.iter().all(|b| b.abs() > m.lattice_max_ns);
        if resolvable {
            let positive = biases[0] > 0.0;
            assert!(
                biases.iter().all(|b| (*b > 0.0) == positive),
                "{}: the ordering-bias estimates disagree in SIGN across runs ({biases:?}) while \
                 every one of them is above the instrument's {} ns lattice. The counterbalance's \
                 premise is that this contamination is a stable offset over a quadruple; four \
                 resolvable estimates that cannot agree on its direction do not support that \
                 premise. The cancellation is still arithmetically harmless, but it is no longer \
                 the reason the numbers are trustworthy — say so when reporting, and treat the \
                 null control as the only evidence that the design works",
                r.label,
                m.lattice_max_ns
            );
        } else {
            println!(
                "  NOTE: at least one bias estimate is at or below the {} ns lattice BOUND, so \
                 sign agreement is not asserted — an unresolvable estimate's sign is rounding, not \
                 evidence. Because the lattice is an upper bound, a LOOSE one suppresses this \
                 assertion more often than a tight one would; distinct_ticks is what says whether \
                 that happened. A bias this small also means the counterbalance was not \
                 load-bearing on these runs, which is worth stating rather than assuming.",
                m.lattice_max_ns
            );
        }
    }
}

/// **The lattice figure is a BOUND, and this is where its evidence is adjudicated.**
///
/// # The finding this test exists because of
///
/// Eight sessions of this rung reported `tick_gcd = 1024` seven times and **128** once. Those are
/// not eight readings of a device property that disagreed; they are eight UPPER BOUNDS on one
/// device property, taken over eight different samples. A GCD over durations `t_i = m_i · G`
/// returns `G · gcd(m_1 … m_n)`, which is `G` only when the observed multipliers are setwise
/// coprime — and a fixed-workload dispatch produces durations clustered on a handful of values,
/// whose multipliers routinely share a factor. The seven agreeing sessions were agreeing about
/// their own homogeneity. The step is at most 128 ns; 1024 was never a measurement of it. Rung
/// S1.5 had written the 1024 into its record as an equality, and
/// `sv0_deferred_term_bench.rs`'s CORRECTION section retracts it.
///
/// # What is asserted
///
/// * **The bound is non-vacuous.** A GCD over ONE distinct value is that value — a "bound" carrying
///   no information at all — so at least two distinct values are required unconditionally.
/// * **A thin bound may not widen the gate.** [`sv0_s5_measurement_meets_its_gates`] consults
///   [`Measured::lattice_may_widen`]; this test asserts the same predicate holds WHENEVER the
///   lattice would actually widen the gate on either row. Widening on thin evidence is the one
///   direction in which a degenerate sample flatters a verdict, and it is closed here on its own,
///   where it cannot be waived by an argument about the term.
///
/// # What is only reported
///
/// The evidence count itself when the lattice does not bind. A homogeneous session set is not a
/// defect — it is what a steady dispatch on a steady machine looks like — and reddening a rung for
/// it would punish the good case. The number is printed either way so a reader can see how much
/// the bound rests on.
#[test]
fn sv0_s5_lattice_bound_rests_on_evidence() {
    let m = measured();
    let distinct = m.lattice_distinct.expect("invariant: measured() rejected the None sentinel");
    let may_widen = m.lattice_may_widen();

    println!(
        "VB-SV0-S5 lattice bound: quantum <= {} ns (pooled by GCD over all ten sessions), resting \
         on {distinct} distinct tick values against a floor of {SV0_LATTICE_MIN_DISTINCT_TICKS} — \
         may_widen={may_widen}. A GCD over observed durations is a MULTIPLE of the counter's step, \
         so this is an UPPER BOUND whose tightness is a property of the SAMPLE.",
        m.quantum_max_ns
    );

    assert!(
        distinct >= 2,
        "the lattice bound rests on {distinct} distinct tick value(s). A GCD over a single value IS \
         that value, so this is not a bound at all — it carries exactly zero information about the \
         counter's step, and any gate reading it would be reading the session's own duration. Re-run \
         with a session that actually varies (the `samples` and `tick_span` fields on the printed \
         lines say whether it did)"
    );

    for r in &m.rows {
        let lattice_floor = m.lattice_max_ns / r.central_ns.abs();
        let would_widen = lattice_floor > SV0_S5_SESSION_SPREAD_MAX;
        println!(
            "  [{}]: lattice_floor<={lattice_floor:.4} vs protocol {SV0_S5_SESSION_SPREAD_MAX} — \
             the lattice {} widen the gate",
            r.label,
            if would_widen { "WOULD" } else { "would not" }
        );
        assert!(
            !would_widen || may_widen,
            "S5 LATTICE EVIDENCE [{}]: the transcribed lattice bound ({} ns on a {} ns term, \
             floor <= {lattice_floor:.4}) is COARSER than the {SV0_S5_SESSION_SPREAD_MAX} protocol \
             gate, i.e. it would widen it — but it rests on only {distinct} distinct tick values, \
             below the pre-registered floor of {SV0_LATTICE_MIN_DISTINCT_TICKS}. A bound from a \
             homogeneous sample is a MULTIPLE of the counter's step, so widening a gate with it \
             would be widening it by an artifact. The remedy is more VARIED evidence about the \
             instrument (more sessions, or sessions whose durations range wider — the discarded \
             warm-up session's ticks pool in for exactly this reason), never a lower floor and \
             never a hand-written lattice",
            r.label,
            m.lattice_max_ns,
            r.central_ns
        );
    }
}

/// **The discarded warm-up session, disclosed rather than absorbed.**
///
/// # Why a whole session is discarded
///
/// The first eight-session run produced a `row1_armed1` median of 549376 ns against 12800 / 13312
/// for the two sessions that followed it — 42x, on a protocol whose gate is 10%. The 20-frame
/// in-session warm-up did not touch it, and could not: sessions 2 and 3 ran the same 20 frames and
/// agreed with each other. The cold start is at PROCESS level, so the discard has to be at process
/// level too. `scripts\sv0_s5_bench.ps1` therefore runs one warm-up session per row, first, on the
/// same command line as an armed session — discarded by BOOKKEEPING, not by a different
/// configuration, so it exercises exactly the code path the kept sessions do.
///
/// The rejected alternative is worth naming: more sessions plus a robust statistic across them
/// would have ABSORBED the cold session into a median and produced a clean-looking number. This
/// whole finding exists only because the harness refused to absorb a disagreement it could not
/// explain, so absorbing is the failure mode rather than the fix.
///
/// # What is asserted
///
/// That the warm-up session's median is TRANSCRIBED (guaranteed by `measured()`'s completeness
/// check) and that it is a positive delta — a warm-up session that measured nothing was not a
/// warm-up, it was a broken session, and reporting a ratio against it would be reporting noise.
///
/// # What is only reported
///
/// Its ratio to the kept sessions' central value. There is deliberately no threshold: a large ratio
/// is the cold start this design exists for, and a ratio near 1 says the cold start was absent on
/// this run — both are information, neither is a defect, and a pass/fail line here would be fitted
/// to whichever run happened to be in front of the author.
#[test]
fn sv0_s5_warmup_session_is_disclosed() {
    let m = measured();
    for r in &m.rows {
        let ratio = r.warmup_delta / r.central_ns;
        println!(
            "VB-SV0-S5 warm-up [{}]: the DISCARDED first session measured {:.1}ns against the \
             three kept sessions' central {:.1}ns — {ratio:.2}x. It is excluded from the statistic \
             and recorded here so a cold session is visible instead of silently absorbed; its raw \
             TICKS still pool into the instrument's lattice bound, because a cold session is \
             invalid evidence about the TERM and valid evidence about the INSTRUMENT.",
            r.label, r.warmup_delta, r.central_ns
        );
        assert!(
            r.warmup_delta > 0.0,
            "{}: the discarded warm-up session's median paired delta is {} ns, i.e. that session \
             saw no term at all. That is not a cold start — it is a session that measured nothing \
             (a clamped SV0 request, or a blind instrument), and the ratio printed beside it would \
             be meaningless. Check that session's log for the `boyko_render: VB-SV0 was requested` \
             clamp line before trusting the three that followed it, since they ran the same \
             configuration",
            r.label,
            r.warmup_delta
        );
    }
}

/// **Row identity is an OBSERVATION, not an assumption.**
///
/// §6 S5 names two rows by their `.spv`, and which producer a run binds is decided from four
/// inputs this test does not see. "I set `BOYKO_SV0_SSAO`, therefore row 7 ran" is a gate
/// quantified over a row nobody verified — this campaign's signature defect, and the exact reason
/// `note_vb_lit_producer` exists (it identifies the row from the BOUND PIPELINE HANDLE, not from
/// the selector). So the transcribed producer name and the summary line's own `row=` label must
/// BOTH match, and they must disagree with each other if the operator mixed up two sessions' logs.
#[test]
fn sv0_s5_row_identity_is_observed() {
    let m = measured();
    for r in &m.rows {
        assert_eq!(
            r.row_label, r.expect_row_label,
            "{}: the summary line printed row={:?}, but this row's sessions must run the {:?} \
             tail. Either BOYKO_SV0_SSAO was set/unset the wrong way for these sessions, or two \
             logs were transcribed into the wrong constants",
            r.label, r.row_label, r.expect_row_label
        );
        assert_eq!(
            r.producer, r.expect_producer,
            "{}: the recorder logged `VB lit producer = {}`, not `{}`. The number transcribed for \
             this row was therefore measured on a DIFFERENT `.spv` than §6 S5 names — a stray \
             BOYKO_SV0_FROXEL would select the _froxel rows, and a textured material would select \
             the _tex ones. Re-run this row with a clean environment",
            r.label, r.producer, r.expect_producer
        );
        println!(
            "VB-SV0-S5 row identity [{}]: row={} producer={} — observed from the bound pipeline \
             handle, not from the selector",
            r.label, r.row_label, r.producer
        );
    }
}

// ===========================================================================================
// The §7 clause 3 adjudication
// ===========================================================================================

/// The fixture's projection — `sv0_adequacy.rs::scene_view_proj_rows` verbatim.
///
/// Duplicated rather than shared because these live in `tests/` binaries that cannot import each
/// other; the duplication is made safe by
/// [`sv0_s5_confound_deflation_is_derived_not_assumed`]'s assertion that this file's coverage
/// count equals the one S1's oracle recorded — the same discipline `sv0_deferred_term_bench.rs`
/// applies to its own copy. Only the COVERAGE half is duplicated here: the ray generation and
/// analytic sphere root that produce the confound NUMERATOR stay in S1.5's file, one
/// implementation, because a third copy of that geometry is how two instruments silently drift.
fn scene_view_proj_rows() -> [[f32; 4]; 4] {
    let view = ViewUniform::from_camera(
        sv0_scene::camera_transform().to_affine(),
        sv0_scene::camera_projection(),
    );
    boyko_render::forward_view_proj_rows(&view, sv0_scene::DUMP_EXTENT, sv0_scene::DUMP_EXTENT)
}

/// The fixture's raster coverage — `sv0_adequacy.rs::scene_coverage` verbatim.
fn scene_coverage() -> Coverage {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let oracle_verts: Vec<OracleVertex> = verts
        .iter()
        .map(|v: &Vertex| OracleVertex { position: v.position, normal: v.normal })
        .collect();
    let instances: Vec<[f32; 3]> =
        (0..sv0_scene::MESH_ROW_COUNT).map(sv0_scene::mesh_center).collect();

    sv0_oracle::rasterize(
        &oracle_verts,
        &idx,
        &instances,
        scene_view_proj_rows(),
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
        sv0_scene::CAMERA_NEAR,
    )
}

/// The deflated reference §7 clause 3 must be read against, and the abort threshold it implies.
struct Reference {
    /// S1.5's number as measured — the cost of BOTH `pc.lighting_flags`-gated arms.
    raw_ns: f64,
    /// `sdf_visible / covered` — the fraction of the reference SV0 does NOT mirror.
    confound_ratio: f64,
    /// `1 / (1 + confound_ratio)` — an UPPER bound on the deflation factor.
    deflation: f64,
    /// `raw_ns * deflation` — the `!own_pixel` share, i.e. the term SV0 actually inlines.
    deflated_ns: f64,
    /// `SV0_S5_ABORT_RATIO * deflated_ns` — above this, §7 clause 3 says revert.
    abort_ns: f64,
}

/// Builds the deflated reference, re-deriving every step that can be re-derived.
fn reference() -> Reference {
    let covered = SV0_SCENE_COVERED_MESH_PIXELS as f64;
    let confound_ratio = SV0_S1_5_CONFOUND_SDF_VISIBLE_PX as f64 / covered;
    let deflation = 1.0 / (1.0 + confound_ratio);
    let deflated_ns = SV0_DEFERRED_TERM_REFERENCE_NS * deflation;
    Reference {
        raw_ns: SV0_DEFERRED_TERM_REFERENCE_NS,
        confound_ratio,
        deflation,
        deflated_ns,
        abort_ns: SV0_S5_ABORT_RATIO * deflated_ns,
    }
}

/// **The deflation is DERIVED, and its direction is asserted.**
///
/// §7 clause 3's threshold is `2 ×` S1.5's reference — and that reference over-states the term SV0
/// mirrors, because `pc.lighting_flags` gates two arms and S1.5's A/B switched both. An inflated
/// reference inflates the threshold, i.e. it lets a MORE expensive SV0 ship. That is the
/// false-GREEN direction, so this test does three things rather than trusting a copied factor:
///
/// 1. **Re-derives the denominator.** The covered-mesh-pixel count is recomputed here from the
///    shipped oracle over the shipped scene, and asserted equal to the transcribed
///    [`SV0_SCENE_COVERED_MESH_PIXELS`]. If the fixture ever moves, this reds instead of quietly
///    re-scaling the threshold.
/// 2. **Computes the ratio and the deflation in code**, from the two pixel counts. No factor is
///    written down anywhere for a later reader to "fix".
/// 3. **Asserts the direction**: the deflation is strictly below 1, so the deflated threshold is
///    strictly below the undeflated one. That is the assertion that would have caught applying the
///    correction the wrong way round, which is the only way this arithmetic can be wrong and still
///    look right.
///
/// ⚠️ It is an **upper bound on the deflation factor**, not an estimate: `1/(1+ratio)` assumes the
/// two arms cost the same per pixel, and the `own_pixel` arm marches from an SDF surface hit. If
/// it is dearer, the true `!own_pixel` share is smaller and the threshold should be lower still.
/// The bound therefore errs toward PERMITTING, which is why a result inside a few percent of the
/// threshold is not a pass anyone should read as comfortable.
///
/// Also printed, and read by nothing: S4's changed-pixel fractions. They are the reason "per
/// armed pixel" is not the comparison — see this file's module doc, "Correction 2".
#[test]
fn sv0_s5_confound_deflation_is_derived_not_assumed() {
    let coverage = scene_coverage();
    assert_eq!(
        coverage.near_rejected_triangles, 0,
        "the fixture's raster must not silently drop near-plane triangles"
    );
    assert_eq!(
        coverage.covered_count(),
        SV0_SCENE_COVERED_MESH_PIXELS,
        "this file's projection/coverage helpers no longer reproduce the raster S1's gates — and \
         S1.5's confound ratio — are quantified over. The deflation below would be computed \
         against a different scene than the reference it corrects"
    );

    let r = reference();
    println!(
        "VB-SV0-S5 reference: raw={:.1}ns (S1.5, BOTH lighting_flags arms) \
         confound sdf_visible_px={} covered_mesh_px={} ratio={:.4} deflation={:.4} \
         deflated={:.1}ns abort_above={:.1}ns ({}x)",
        r.raw_ns,
        SV0_S1_5_CONFOUND_SDF_VISIBLE_PX,
        SV0_SCENE_COVERED_MESH_PIXELS,
        r.confound_ratio,
        r.deflation,
        r.deflated_ns,
        r.abort_ns,
        SV0_S5_ABORT_RATIO
    );
    println!(
        "  Context, read by NO gate: S4 gate (ii) measured CHANGED-pixel fractions of \
         {:.2}–{:.2}% (shadow) and {:.2}–{:.2}% (contact AO) of the {} covered mesh pixels. Those \
         are pixels whose OUTPUT MOVED, not pixels that RAN the term — the shipped blocks execute \
         on essentially every covered pixel — so they are not a denominator for a cost. The \
         comparison below is per-FRAME nanoseconds, both sides on this fixture at 512x512.",
        SV0_S4_SHADOW_CHANGED_FRACTION_RANGE[0] * 100.0,
        SV0_S4_SHADOW_CHANGED_FRACTION_RANGE[1] * 100.0,
        SV0_S4_AO_CHANGED_FRACTION_RANGE[0] * 100.0,
        SV0_S4_AO_CHANGED_FRACTION_RANGE[1] * 100.0,
        SV0_SCENE_COVERED_MESH_PIXELS
    );

    assert!(
        r.confound_ratio > 0.0,
        "the confound ratio is {}, i.e. the SDF body is invisible on this fixture — which \
         contradicts sv0_scene's own placement derivation (a ~66 px disc) and would mean the \
         reference needs no deflation at all",
        r.confound_ratio
    );
    assert!(
        r.deflation < 1.0 && r.deflation > 0.0,
        "the deflation factor is {}, which does not DEFLATE. An inflated reference RAISES §7 \
         clause 3's abort threshold — the false-GREEN direction — so the correction must be \
         applied as 1/(1+ratio) and never as (1+ratio)",
        r.deflation
    );
    assert!(
        r.abort_ns < SV0_S5_ABORT_RATIO * r.raw_ns,
        "the deflated abort threshold ({}) is not below the undeflated one ({}); the correction \
         was applied in the direction that lets a MORE expensive SV0 ship",
        r.abort_ns,
        SV0_S5_ABORT_RATIO * r.raw_ns
    );
}

/// **§7 clause 3, adjudicated.** The verdict this whole rung exists to produce.
///
/// > *"S5's median paired delta exceeds **2×** S1.5's measured `SV0_DEFERRED_TERM_REFERENCE` on
/// > the same fixture. … In `[1×, 2×]` it ships with the number recorded; above 2×, revert."*
///
/// Read against the DEFLATED reference ([`sv0_s5_confound_deflation_is_derived_not_assumed`]), per
/// row, in per-frame nanoseconds on the same fixture at the same extent (see the module doc for
/// why per-frame and not per-armed-pixel). The worse of the two rows governs: §6 S5 names both
/// tails precisely because they are structurally different, so a stage that is affordable on the
/// fused tail and not on the split one has not passed.
///
/// The `[1×, 2×]` band ships **with the number recorded** — so the ratio is printed on every run,
/// pass or fail, and this test's output is the record.
#[test]
fn sv0_s5_cost_clause_is_adjudicated() {
    let m = measured();
    let r = reference();

    let mut worst_ratio = 0.0f64;
    let mut worst_label = "";
    for row in &m.rows {
        let ratio = row.central_ns / r.deflated_ns;
        let band = if ratio <= 1.0 {
            "below 1x — cheaper than the shipped Deferred sibling"
        } else if ratio <= SV0_S5_ABORT_RATIO {
            "in [1x, 2x] — SHIPS, with the number recorded (§7 clause 3)"
        } else {
            "ABOVE 2x — §7 clause 3 says REVERT"
        };
        println!(
            "VB-SV0-S5 clause 3 [{}]: median={:.1}ns vs deflated reference {:.1}ns => {:.3}x \
             (abort above {:.1}ns) — {band}",
            row.label, row.central_ns, r.deflated_ns, ratio, r.abort_ns
        );
        if ratio > worst_ratio {
            worst_ratio = ratio;
            worst_label = row.label;
        }
    }

    assert!(
        worst_ratio <= SV0_S5_ABORT_RATIO,
        "§7 CLAUSE 3 ABORT: {worst_label}'s median paired delta is {worst_ratio:.3}x the DEFLATED \
         SV0_DEFERRED_TERM_REFERENCE ({:.1}ns = {:.1}ns x {:.4}), above the {}x threshold \
         ({:.1}ns). The stage is REVERTED — not softened, not re-scoped mid-flight. Revert \
         granularity is the plan's own: S2-S4 come out, S0 and S1 stay (the OFF-path harness, the \
         fixture, the coverage oracle and the changed-pixel comparator are a gain regardless). \
         Note the threshold is generous by construction: the deflation is an UPPER bound (it \
         assumes both lighting_flags arms cost the same per pixel), so the true threshold is lower \
         than this one and the overrun is at least this large",
        r.deflated_ns,
        r.raw_ns,
        r.deflation,
        SV0_S5_ABORT_RATIO,
        r.abort_ns
    );
    println!(
        "VB-SV0-S5 clause 3 VERDICT: worst row {worst_label} at {worst_ratio:.3}x the deflated \
         reference — within the {}x abort threshold. Record this number in the commit.",
        SV0_S5_ABORT_RATIO
    );
}
