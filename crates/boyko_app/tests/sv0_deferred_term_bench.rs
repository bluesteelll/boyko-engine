//! ⚠️ **Profiling rung 7 — this file's migration is a RELABELLING, not a re-pointing.** The corpus's
//! consumer list says it *"reads stdout"* and must be migrated to read the artifact. MEASURED
//! before a line was changed: it contains **zero** `Command::new`, `.output()` or `stdout` — it
//! never reads a child at runtime at all. What it holds are TRANSCRIPTIONS of lines the harness
//! printed, checked by device-free arithmetic. So rung 7 does not re-point it; rung 7 retires the
//! producer it documents, which makes its numbers a record of a retired instrument and its
//! invocation recipe below obsolete at the deletion commit. Same disposition as
//! `vb_bench_totality_gate.rs`, reached for the same reason and by the same measurement: the second
//! consumer list described what these files were ABOUT rather than what they DO.
//!
//! **VB-SV0 rung S1.5 — the cost falsifier** (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §6 "S1.5"): a
//! COUNTERBALANCED interleaved A/B of the SHIPPED Deferred SDF soft-shadow + contact-AO term, on
//! S1's fixture scene, with ZERO new shader code.
//!
//! # What it measures, and why that is the term SV0 would inline
//!
//! `sdf_gbuffer_composite.hlsl` gates BOTH of its shadow/AO march sites on the SAME push word,
//! `pc.lighting_flags`. Alternating that word between `SHADOWS|AO` and `0` on adjacent frames
//! therefore switches exactly those marches on and off, and nothing else: the flags live in
//! `FineMarcherPush` (offset 8), a per-frame PUSH CONSTANT — not a descriptor, not a pipeline
//! key — so the two phases of a pair share one pipeline, one descriptor set, one recorded
//! command stream shape, and differ in four bytes. (`GBufferScene::lighting_flags`' own doc
//! states this property; this bench is the first consumer to depend on it.)
//!
//! The marcher dispatch is bracketed by a dedicated one-pass GPU-timestamp collector
//! (`Sv0TimestampCollector`, armed only under `BOYKO_SV0_BENCH` — both DELETED at profiling rung 7
//! step 6c), so the reported number was GPU wall-clock for that dispatch, and the PAIRED
//! difference is what cancels the bracket's own drain/overlap bias.
//!
//! # ⚠️ The ABAB design was REFUTED by its own null control — read this before the numbers
//!
//! Revision 1 of this harness alternated strictly, A,B,A,B. Three armed sessions reported medians
//! of 5632 / 6144 / 6144 ns — a tidy 8.3% cross-session spread, inside the 10% gate. The null
//! control (both phases pushing the ARMED word, so the true difference is exactly zero) reported
//! **−2048 ns**: a THIRD of the "signal", with `p10 = −7168` and `p90 = +4096`, i.e. the entire
//! band shifted below zero. A drift that random would scatter around zero. This did not. It was a
//! constant ORDERING bias, and ABAB cannot remove one.
//!
//! ## Why ABAB leaves it in every delta, with the same sign
//!
//! Model a sample at cycle position `k`:
//!
//! ```text
//! m_k = mu + tau * armed(k) + gamma(fi(k)) + beta * k + eps_k
//! ```
//!
//! `tau` is the term under measurement; `gamma` a per-frame-in-flight-slot offset; `beta` a local
//! position slope; `eps` zero-mean noise. Under ABAB every delta is
//!
//! ```text
//! m_k - m_{k+1} = tau + (gamma_f - gamma_{1-f}) - beta
//! ```
//!
//! — the same contamination, the same sign, in all forty of them. A median over deltas that each
//! carry an identical offset returns that offset. It removes outliers, not bias.
//!
//! `gamma` is not hypothetical on this engine. `FRAMES_IN_FLIGHT == 2`, so under ABAB the A/B
//! phase is PERFECTLY ALIASED with the frame-in-flight slot: ARMED always landed on `fi = 0` and
//! CLEARED always on `fi = 1` — a different query pool, a different descriptor/UBO ring slot, a
//! different staging region, every single frame. The A/B was confounded with the ring by
//! construction, and no amount of sampling could have separated them.
//!
//! ## Why the ABBA quadruple removes it
//!
//! Frames run A,B,B,A. Over positions `k..k+3`, with ring slots `f, 1-f, f, 1-f`:
//!
//! ```text
//! d1 = m_k     - m_{k+1} = tau + (gamma_f - gamma_{1-f}) - beta
//! d2 = m_{k+3} - m_{k+2} = tau - (gamma_f - gamma_{1-f}) + beta
//!
//! DELTA = (d1 + d2) / 2 = tau                              <- the statistic
//! BIAS  = (d1 - d2) / 2 = (gamma_f - gamma_{1-f}) - beta   <- reported, not hidden
//! ```
//!
//! Both contaminations cancel EXACTLY, because the second half of the quadruple presents them
//! with the opposite sign. Each phase now takes one sample on each ring slot per quadruple, so the
//! `fi` alias is BROKEN rather than averaged — and that holds whatever slot the quadruple starts
//! on, which matters because a dropped frame can rotate the phase-to-slot alignment mid-session.
//!
//! `BIAS` is exactly what ABAB was adding to every delta, so the harness PRINTS it
//! (`median_order_bias_ns`, with its own p10/p90 band). A design that quietly averages a bias away
//! leaves no way to tell whether the bias was stable enough for the averaging to be sound;
//! [`sv0_s1_5_order_bias_is_reported`] reads it across all four runs for that reason.
//!
//! **What ABBA does NOT remove:** a position effect with non-zero SECOND difference. For a purely
//! quadratic `c·k²` the residual is `2c`. That is precisely what the null control bounds, which is
//! why the null control survives this redesign instead of being retired by it.
//!
//! The within-quadruple mean of two paired deltas is NOT the "difference of means" §6 S1.5
//! excludes: both terms are already PAIRED differences of adjacent frames, and their mean is the
//! algebra above. The session statistic remains a MEDIAN, over quadruples.
//!
//! # ⚠️ The instrument is QUANTISED, and the spread gate has to know it
//!
//! Every number Revision 1 printed was a multiple of 512 ns — and every single-order statistic
//! (`p10`/`p90`: 1024, 12288, 0, 3072, −7168, 4096) was a multiple of **1024**. Only the medians
//! showed 512s, which is what an even-count median does when it averages two adjacent order
//! statistics. So no observed value contradicted a 1024 ns lattice, and the reported median's was
//! read as 512 ns.
//!
//! The lattice is a hardware property no Vulkan limit reports — `timestampPeriod` is the
//! ns-per-tick SCALE, not the STEP — so this revision measures what it can of it: the runner reads
//! raw timestamp TICKS (never period-scaled floats, which cannot evidence an integer lattice) and
//! takes their GCD over the whole session, printing `tick_gcd`, `distinct_ticks`, `min_tick_gap`,
//! `tick_span`, `quantum_max_ns`, `median_lattice_max_ns`, `timestamp_period_ns`,
//! `timestamp_valid_bits` and `timestamp_compute_and_graphics` on its `RESOLUTION:` line. Those
//! are transcribed below and the spread gate is read against them.
//!
//! # ⚠️ CORRECTION — the 1024 ns figure was a BOUND, and a loose one. Two claims below were wrong
//!
//! **What this file said before, and what is now known.** The paragraphs above once concluded "the
//! raw lattice **was** 1024 ns", and a paragraph further down concluded that Revision 1's 8.3%
//! cross-session spread was "ONE median lattice step … the gate was measuring the smallest non-zero
//! number the instrument can print". Rung S5 falsified both, and this section is the correction.
//!
//! **The mechanism.** A GCD taken over observed durations is `G · gcd(m_1 … m_n)` for
//! `t_i = m_i · G`. It equals the hardware step `G` only when the observed multipliers are setwise
//! coprime; otherwise it is a MULTIPLE of `G` and the number itself does not say which. So the
//! estimator can only ever license `G <= quantum`, and its tightness is a property of THE SAMPLE.
//!
//! **The evidence.** S5 ran eight sessions of the same protocol on the same device. Seven reported
//! `tick_gcd = 1024`; exactly one reported **128**. The odd one out was the first session of a
//! fresh process set — the one whose durations ranged widest. A fixed-workload dispatch produces
//! durations clustered on a handful of values; a handful of clustered multipliers routinely share
//! a factor. The seven agreeing sessions were agreeing about their own homogeneity.
//!
//! So the device's step is **at most 128 ns**, possibly less, and 1024 was never a measurement of
//! it. Note which session supplied the tighter bound: a cold session is invalid evidence about the
//! TERM and perfectly valid evidence about the INSTRUMENT, because its durations are integer tick
//! counts on the same counter whatever the clocks were doing.
//!
//! **What that does to the two wrong claims.**
//!
//! * "The 8.3% spread was one lattice step." **False.** At `quantum <= 128` the reported median's
//!   lattice is `<= 32 ns`, so Revision 1's 512 ns spread was at least SIXTEEN lattice steps and
//!   its 6144 ns signal was at least ~48 quanta wide, not six. That spread was REAL session
//!   variation. The instrument was never the thing being measured there.
//! * "`256 / 6144 = 4.2%` is the resolution floor this gate sits above." **Loose, in the
//!   flattering direction.** The true floor is `<= 32 / 6144 = 0.5%`. The gate's VERDICT is
//!   unchanged either way — 4.2% and 0.5% are both under the 10% protocol tolerance, so the
//!   lattice term never bound and `effective_max` was 10% in both readings — but a reader who took
//!   4.2% for the instrument's resolution was told the instrument was eight times blunter than it
//!   is.
//!
//! **The direction of the error, stated plainly.** The instrument is FINER than this file claimed.
//! That is the safe direction for every number already transcribed here: nothing that passed
//! should have failed. It is the unsafe direction for INTERPRETATION, because an overstated
//! quantum is exactly what excuses a real spread as quantisation — which is what the superseded
//! sentence did.
//!
//! **What was changed, and what deliberately was not.** The MEASURED literals below are untouched:
//! they are what these sessions measured, and a measured literal is re-measured, never adjusted.
//! [`SV0_S1_5_QUANTUM_MAX_NS`] and [`SV0_S1_5_MEDIAN_LATTICE_MAX_NS`] are the same numbers under
//! names that say "bound". What changed is (a) the runner now prints the evidence a bound rests on
//! (`distinct_ticks`, `min_tick_gap`, `tick_span`), (b) these sessions predate that field so
//! [`SV0_S1_5_QUANTUM_DISTINCT_TICKS`] is `None`, and (c) a bound with no recorded evidence — or
//! with too little — is NOT ALLOWED TO WIDEN the spread gate (see
//! [`sv0_s1_5_measurement_meets_its_gates`]). Re-measuring this rung under the corrected harness is
//! what replaces the literals; until then the coarse bound is inert rather than flattering.
//!
//! ## What was done about it, and what was deliberately NOT done
//!
//! Two things, neither of which touches the measured scene:
//!
//! 1. **The counterbalance halves the lattice, twice.** A quadruple statistic is `(d1 + d2)/2` —
//!    a half-sum of two multiples of the quantum, so it lands on a `quantum/2` lattice; an even
//!    quadruple count halves that again at the median. At the bound these sessions recorded
//!    (`quantum <= 1024 ns`) the reported median's lattice is `<= 256 ns`, i.e. `<= 4.2%` of the
//!    signal — under the 10% gate, so the gate can bind on something real. Under S5's tighter
//!    `quantum <= 128 ns` it is `<= 0.5%`, so this change bought far more headroom than it was
//!    credited with; see the CORRECTION section for why the 1024 was a loose bound. This is
//!    genuine dithering, not an arithmetic trick: the per-pair band spans dozens of quanta, so the
//!    two deltas being averaged routinely fall on DIFFERENT lattice points. (Were every sample
//!    identical it would buy nothing, and `distinct_ticks` on the `RESOLUTION:` line would say so.)
//! 2. **The sample size is raised from 40 to 200.** The median's sampling SE is `1.253·σ/√n`;
//!    Revision 1's `p10..p90` band implies `σ ≈ 4.8 µs`, so at `n = 40` the session median carried
//!    an SE near 15% of the signal — larger than the 10% gate it was read against, meaning a
//!    passing spread was as much luck as evidence. Halving the per-unit variance (the quadruple
//!    averages two deltas) and taking `n = 200` puts the SE near 3%. 800 frames is a few seconds
//!    and changes NOTHING about what is measured.
//!
//! **NOT done: raising the signal by changing the scene.** More SDF bodies, a larger render
//! extent, or more marcher work per pixel would each raise the term against the quantum — and each
//! would measure something S1's adequacy oracle never certified. Worse, §7 clause 3 compares S5's
//! delta against `2×` this reference, so S5 would have to be moved to the same altered scene or
//! the comparison is meaningless. The two changes above buy the needed resolution at zero cost to
//! what the number MEANS, so the scene stays exactly as certified. If the measured lattice BOUND
//! turns out coarser than the 1024 ns inferred above, [`sv0_s1_5_instrument_resolves_its_signal`]
//! goes RED and says so in those terms — a scene change is then the remedy of last resort, and it
//! must be applied to S5 in the same breath.
//!
//! # ⚠️ The plan's transfer claim is INEXACT, in the false-GREEN direction
//!
//! §6 S1.5 says the A/B is gated "at `sdf_gbuffer_composite.hlsl:1865` by `pc.lighting_flags`",
//! i.e. the `!own_pixel` raster-owned arm that writes `gMaterial.RG = (mesh_shadow, mesh_ao)` —
//! the arm SV0 mirrors. That is one of TWO arms the word gates. The other is the `own_pixel`
//! SDF-hit arm at `:1805`, which runs the same two leaves on every pixel where the SDF surface is
//! visible in front of the mesh. Clearing the word switches off BOTH.
//!
//! So the measured delta is
//!
//! ```text
//! delta = cost(mesh arm over covered mesh pixels) + cost(SDF arm over SDF-visible pixels)
//! ```
//!
//! and it OVER-states the `!own_pixel` term that SV0 actually mirrors. The plan calls the number
//! "a lower bound on SV0's cost, not an estimate of it", reasoning that under VB every covered
//! pixel is a mesh pixel so SV0's coverage is a superset of the `!own_pixel` arm's. That
//! coverage argument holds (and on this fixture the two sets are in fact EQUAL — S1's oracle
//! asserts `MeshSelection::sdf_occluded == 0`, i.e. the body eclipses no mesh pixel), but the
//! measured quantity is not the `!own_pixel` arm alone, so "lower bound" does not follow from it.
//!
//! Why the direction matters: §7 clause 3 aborts when S5's delta exceeds **2×** this reference.
//! An inflated reference inflates the threshold, i.e. it lets a MORE expensive SV0 ship. That is
//! the false-GREEN direction, which is why the confound is quantified here rather than noted in
//! prose: [`sv0_s1_5_confound_set_is_bounded`] measures the SDF-visible pixel count against the
//! covered mesh pixel count on the CPU, so the over-statement has a number attached and the
//! orchestrator can deflate the reference before adjudicating clause 3.
//!
//! It cannot be removed without a shader edit (the two arms share one gate word and SV0's own
//! §3.1 two-bit field does not exist yet), and it cannot be removed by moving the body (that
//! changes the field, hence both arms' march costs). It is measured, not eliminated.
//!
//! # Protocol — non-negotiable (§6 S1.5, the VB-P1d lesson)
//!
//! Sequential before/after measured a phantom regression on this exact hardware that was entirely
//! session drift, and the VB-P1d bench was later found not to reproduce above `N_ps` ≈ 128 with
//! ~21% run-to-run spread. Therefore:
//!
//! * INTERLEAVED, and now COUNTERBALANCED (A,B,B,A) — never all-A then all-B, and never a strict
//!   alternation that aliases the phase against the frame-in-flight ring. The runner decides each
//!   frame's cycle position BEFORE recording it and tags every stored sample with the position it
//!   ran under, so a dropped frame orphans a whole quadruple instead of mis-signing a later one.
//! * warm-up discarded (`boyko_app`'s `SV0_BENCH_WARMUP`, a multiple of the cycle length).
//! * ≥ [`SV0_BENCH_MIN_QUADS`] quadruples — the floor applied at the level of the STATISTIC's own
//!   sample size, which is strictly stronger than the plan's ≥ [`SV0_BENCH_MIN_PAIRS`] pairs (one
//!   quadruple contains two paired deltas).
//! * the statistic is the **median paired delta**, not a difference of means.
//! * repeated across [`SV0_BENCH_SESSIONS`] separate processes, with the cross-session spread
//!   reported and gated at [`SV0_SESSION_SPREAD_MAX`] **or the instrument's own measured lattice,
//!   whichever is coarser** — with the lattice term itself gated by
//!   [`sv0_s1_5_instrument_resolves_its_signal`], so it can never silently widen the gate.
//!
//! # Env knobs
//!
//! - `BOYKO_SV0_BENCH=1` (any value) — arms the marcher timestamp collector + the runner's ABBA
//!   A/B loop. Unset ⇒ this test behaves exactly like an ordinary windowed run: no collector, no
//!   reset/write commands, `lighting_flags` keeps its shipped literal — a byte-identical command
//!   stream and no golden moves.
//! - `BOYKO_SV0_BENCH_QUADS=<n>` — the TIMED quadruple budget (default 200 in `boyko_app::runner`).
//! - `BOYKO_SV0_BENCH_NULL=1` (any value) — **the null control.** Both phases push the ARMED
//!   flags, so the two configurations are IDENTICAL and the reported median paired delta is pure
//!   residual. §7 clause 5 requires it be judged numerically, against
//!   [`SV0_NULL_CONTROL_MAX_FRACTION`] of the armed median — pre-registered here, before the run.
//!
//! # Runbook (the orchestrator runs this; every session is a separate PROCESS)
//!
//! Three ARMED sessions, run one at a time:
//!
//! ```text
//! $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-gnu'
//! $env:BOYKO_DISABLE_VALIDATION='1'; $env:BOYKO_SV0_BENCH='1'
//! Remove-Item Env:\BOYKO_SV0_BENCH_NULL -ErrorAction SilentlyContinue
//! cargo test -p boyko-app --test sv0_deferred_term_bench sv0_deferred_term_bench `
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! then ONE null-control session, identical but with `$env:BOYKO_SV0_BENCH_NULL='1'` added.
//!
//! Each session prints two transcribable lines:
//!
//! ```text
//! VB-SV0-S1.5 mode=armed quads=… samples=… extent=…x… median_delta_ns=…
//!             median_order_bias_ns=… median_armed_ns=… median_cleared_ns=…
//!             p10_delta_ns=… p90_delta_ns=… p10_bias_ns=… p90_bias_ns=…
//!             median_delta_first_half_ns=… median_delta_second_half_ns=…
//! VB-SV0-S1.5 RESOLUTION: timestamp_period_ns=… tick_gcd=… distinct_ticks=…
//!             min_tick_gap=… tick_span=… quantum_max_ns=… median_lattice_max_ns=…
//!             timestamp_valid_bits=… timestamp_compute_and_graphics=…
//! ```
//!
//! Transcribe, into the MEASURED block below:
//!
//! | printed field | destination |
//! |---|---|
//! | armed `median_delta_ns` ×3 | [`SV0_S1_5_SESSION_MEDIAN_DELTA_NS`] |
//! | armed `quads` ×3 | [`SV0_S1_5_SESSION_QUADS`] |
//! | armed `median_order_bias_ns` ×3 | [`SV0_S1_5_SESSION_ORDER_BIAS_NS`] |
//! | null `median_delta_ns` | [`SV0_S1_5_NULL_MEDIAN_DELTA_NS`] |
//! | null `median_order_bias_ns` | [`SV0_S1_5_NULL_ORDER_BIAS_NS`] |
//! | `timestamp_period_ns` | [`SV0_S1_5_TIMESTAMP_PERIOD_NS`] |
//! | `quantum_max_ns`, POOLED by GCD over all four sessions | [`SV0_S1_5_QUANTUM_MAX_NS`] |
//! | `median_lattice_max_ns`, recomputed from the pooled bound | [`SV0_S1_5_MEDIAN_LATTICE_MAX_NS`] |
//! | `distinct_ticks` of the session that supplied the pooled bound | [`SV0_S1_5_QUANTUM_DISTINCT_TICKS`] |
//! | median of the three armed medians | [`SV0_DEFERRED_TERM_REFERENCE_NS`] |
//!
//! ⚠️ **The `RESOLUTION:` line states BOUNDS, so sessions that disagree are not contradicting each
//! other.** `quantum_max_ns` is `G · gcd(observed multipliers)` — a MULTIPLE of the device's step
//! whenever the sample is homogeneous. Four sessions therefore yield four upper bounds on ONE
//! device property, and the strongest statement they jointly support is their **GCD**: not a
//! majority vote, and not "pick one". Transcribe the pooled value together with the
//! `distinct_ticks` of the session that produced it, because a bound with no evidence behind it is
//! refused as a gate-widener downstream. `timestamp_period_ns`, `timestamp_valid_bits` and
//! `timestamp_compute_and_graphics` ARE flat device properties and must agree exactly; a
//! disagreement THERE is a real finding.
//!
//! Also CHECK, and report rather than transcribe: that `extent` reads `512x512` on every session
//! (an OS-clamped window would silently measure a different per-pixel workload); that
//! `samples` is close to `4 * quads` (a large shortfall means the stream was dropping frames); and
//! that each session's `median_delta_first_half_ns` and `median_delta_second_half_ns` agree with
//! each other — they are the in-session ramp discriminator, and halves that disagree mean the
//! session was still settling while it recorded. Cold starts at the SESSION level are handled the
//! way rung S5's runbook handles them: with a DISCARDED warm-up session, run first.
//!
//! Then run the CPU gates:
//!
//! ```text
//! cargo test -p boyko-app --test sv0_deferred_term_bench -- --nocapture
//! ```
//!
//! Windowed-test conventions (mirrors `vb_p1d_cull_shade_bench.rs`): `#[ignore]` (needs a real
//! windowed GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.

#![cfg(windows)]

// Profiling rung 7 step 6c deleted this file's windowed driver, so nothing here boots an engine
// any more. What is left is arithmetic over transcribed numbers plus the coverage oracle those
// numbers' confound analysis rests on — the imports shrank to match.
use boyko_render::mesh::Vertex;
use boyko_scene::ViewUniform;

mod sv0_oracle;
mod sv0_scene;

use sv0_oracle::{Coverage, OracleVertex};

// ===========================================================================================
// Pre-registered protocol constants — fixed BEFORE any measurement exists
// ===========================================================================================

/// The plan's pair floor (§6 S1.5: "≥ 30 pairs"). A session below it is not a valid sample of
/// this protocol regardless of how tight its numbers look.
const SV0_BENCH_MIN_PAIRS: usize = 30;

/// The floor this harness actually enforces, applied at the level of the STATISTIC's own sample
/// size: ≥ 30 completed ABBA QUADRUPLES.
///
/// Strictly stronger than [`SV0_BENCH_MIN_PAIRS`], not a reinterpretation of it — one quadruple
/// contains TWO paired deltas, so 30 quadruples is 60 pairs. The plan's floor is stated in pairs
/// because Revision 1's unit was a pair; the counterbalanced unit is a quadruple, and a floor that
/// counted pairs would let 15 quadruples through while the median it gates had only 15 samples.
const SV0_BENCH_MIN_QUADS: usize = SV0_BENCH_MIN_PAIRS;

/// The plan's session count (§6 S1.5: "repeated across 3 sessions"). Three separate PROCESSES,
/// not three windows of one process: the failure mode being guarded against is per-process GPU
/// clock/power state, which a single process cannot resample.
const SV0_BENCH_SESSIONS: usize = 3;

/// **The gate** (§6 S1.5): the relative spread of the median across the sessions must not exceed
/// this. Above it, the instrument is not trustworthy at this scale and §7's cost clause cannot be
/// adjudicated — §7 clause 5, not a dead end.
///
/// "Relative spread" is defined HERE because the plan does not define it: `(max − min) / median`
/// over the [`SV0_BENCH_SESSIONS`] session medians. Peak-to-peak over the central value, i.e. the
/// same shape as the "~21% run-to-run spread" the VB-P1d record quotes (1.29 / 1.33 / 1.57 ms →
/// 0.28/1.33 = 21%), so this gate is commensurable with the precedent that motivated it.
///
/// The EFFECTIVE gate is `max(this, measured median lattice / |median|)` — see
/// [`sv0_s1_5_measurement_meets_its_gates`] for why a gate finer than the instrument's own
/// resolution is unreadable, and [`sv0_s1_5_instrument_resolves_its_signal`] for the separate,
/// non-waivable assertion that the lattice term does NOT bind. Together those make the widening
/// impossible to perform silently: if the lattice binds, a test goes RED and names the fact.
const SV0_SESSION_SPREAD_MAX: f64 = 0.10;

/// **The null control's pre-registered threshold** (§7 clause 5: "`|median paired delta|` on two
/// *identical* configurations must be ≤ a pre-registered fraction of the armed delta, not `~0`").
///
/// Registered at the same 10% as [`SV0_SESSION_SPREAD_MAX`], and for the same reason: a drift
/// floor larger than the gate's own tolerance would make the gate unreadable — a "green" spread
/// could then be entirely drift. Fixed here, before any run, so it cannot be widened to rescue a
/// failing control.
///
/// Revision 1 FAILED this at 33% (−2048 ns against a 6144 ns armed median), which is what
/// produced the counterbalanced design. The number is unchanged; the harness moved.
const SV0_NULL_CONTROL_MAX_FRACTION: f64 = 0.10;

/// **The evidence floor a lattice BOUND must clear before it is allowed to widen the spread gate.**
///
/// The `RESOLUTION:` line's `quantum_max_ns` is `G · gcd(m_1 … m_n)` over the `n` DISTINCT observed
/// durations. Under the generic model — multipliers behaving like independent uniform integers —
/// `P(gcd(m) = 1) = 1/ζ(n)`, so the chance the bound OVERSTATES the step is `1 − 1/ζ(n)`:
/// 1.70% at `n = 6`, **0.83% at `n = 7`**, 0.42% at `n = 8`. Seven is the smallest `n` that puts
/// the overstatement risk under 1%, and that derivation — not any observed count — is where this
/// number comes from.
///
/// Two honesty notes it must be read with. Clustered durations are NOT generic, so this is a floor
/// on the evidence and never a guarantee; and the bound also divides `min_tick_gap`
/// deterministically, so a session whose distinct values sit far apart cannot produce a tight bound
/// however many of them there are. Read both figures off the `RESOLUTION:` line.
///
/// What clearing it licenses is narrow and one-directional: ONLY the widening of
/// [`SV0_SESSION_SPREAD_MAX`] in [`sv0_s1_5_measurement_meets_its_gates`]. A bound that fails it is
/// still printed, still transcribed, and still read by
/// [`sv0_s1_5_instrument_resolves_its_signal`] — it simply cannot make the gate more permissive,
/// which is the only direction a degenerate sample could ever flatter a result.
const SV0_LATTICE_MIN_DISTINCT_TICKS: usize = 7;

// ===========================================================================================
// MEASURED values — do not edit these literals to make a failing run pass
// ===========================================================================================
//
// Every literal in this block is a TRANSCRIPTION of a `VB-SV0-S1.5 …` line the harness printed.
//
// ⚠️ PROFILING RUNG 7 STEP 6c RETIRED THE HARNESS THAT PRINTED THEM. These numbers therefore become a
// record of a measurement taken on an instrument this tree no longer contains — the runner's
// `BOYKO_SV0_BENCH` ABBA loop and its stdout line. They are kept because the FINDINGS they support
// (the quantisation floor, the order-bias bound, the confound set) are about the GPU and the
// protocol rather than about the printer, and because the arithmetic gates below are device-free
// and still run. But by this repository's own rule — a result established on a different instrument
// bounds nothing about this one — **a re-measurement on the artifact channel would produce new
// numbers, not a confirmation of these**, and any rung that needs a CURRENT figure must take one.
// The standing discipline: a measured literal may be RE-measured, never adjusted. If a gate below
// fails, the finding is that the gate failed — the remedy is the fixture, the protocol or an
// abort under §7, never the number.
//
// `f64::NAN` is the UNMEASURED sentinel. It is not a placeholder value that could be mistaken for
// evidence: every comparison against NaN is false, so a forgotten transcription cannot produce a
// passing gate — it produces the explicit "not measured" failure in
// `sv0_s1_5_measurement_meets_its_gates`.

/// The three sessions' `median_delta_ns`, in run order (`mode=armed`).
const SV0_S1_5_SESSION_MEDIAN_DELTA_NS: [f64; SV0_BENCH_SESSIONS] =
    [6144.0, 6144.0, 5632.0];

/// The three sessions' `quads`, in the same run order — transcribed so the ≥
/// [`SV0_BENCH_MIN_QUADS`] floor is quantified over the runs that actually happened rather than
/// over the runner's default.
const SV0_S1_5_SESSION_QUADS: [usize; SV0_BENCH_SESSIONS] = [200, 200, 200];

/// The three sessions' `median_order_bias_ns` (`mode=armed`) — the per-quadruple `(d1 − d2)/2`
/// estimate of the ordering + frame-in-flight-slot contamination the counterbalance cancels.
///
/// Transcribed, not merely printed, because a bias that is cancelled and then forgotten is
/// indistinguishable from a bias that was never there. [`sv0_s1_5_order_bias_is_reported`] reads
/// these against [`SV0_S1_5_NULL_ORDER_BIAS_NS`].
const SV0_S1_5_SESSION_ORDER_BIAS_NS: [f64; SV0_BENCH_SESSIONS] =
    [-512.0, 256.0, 0.0];

/// The null control's `median_delta_ns` (`mode=null`) — the harness measuring two IDENTICAL
/// configurations. Under ABBA this is no longer "drift": the constant ordering bias is cancelled
/// by construction, so what remains is the second-order position effect plus sampling noise.
const SV0_S1_5_NULL_MEDIAN_DELTA_NS: f64 = 512.0;

/// The null control's `median_order_bias_ns`. The position effect does not depend on the A/B
/// word, so this is a FOURTH independent estimate of the same physical quantity as
/// [`SV0_S1_5_SESSION_ORDER_BIAS_NS`] — which is what makes cross-run agreement (or disagreement)
/// evidence rather than decoration.
const SV0_S1_5_NULL_ORDER_BIAS_NS: f64 = 0.0;

/// `VkPhysicalDeviceLimits::timestampPeriod` as the harness read it — nanoseconds per GPU
/// timestamp TICK. The SCALE, not the STEP.
const SV0_S1_5_TIMESTAMP_PERIOD_NS: f64 = 1.0;

/// The measured `quantum_max_ns`: the GCD of every raw per-frame tick count the session read, times
/// [`SV0_S1_5_TIMESTAMP_PERIOD_NS`]. An **UPPER BOUND** on the counter's step, in nanoseconds —
/// `quantum <= 1024`, never `quantum == 1024`.
///
/// ⚠️ Read the module doc's CORRECTION section before using this number for anything. It is a
/// MEASURED literal and is left exactly as these sessions produced it, but the interpretation it
/// once carried ("the counter's actual STEP") was wrong: a GCD over observed durations is a
/// MULTIPLE of the step whenever the sample is homogeneous, and rung S5 later observed **128 ns**
/// on the same device. Every session that produced this 1024 saw a fixed-workload dispatch whose
/// durations clustered on a handful of 1024-multiples.
///
/// The name carries the `MAX` so no reader can restore the equality by accident, and
/// [`SV0_S1_5_QUANTUM_DISTINCT_TICKS`] is what stops the loose bound from widening any gate.
const SV0_S1_5_QUANTUM_MAX_NS: f64 = 1024.0;

/// The measured `median_lattice_max_ns`: an upper bound on the lattice the REPORTED session median
/// lands on, which is `quantum / 2` (each quadruple statistic is a half-sum of two multiples of the
/// quantum) and `quantum / 4` when the quadruple count is even and the median averages two order
/// statistics.
///
/// Bounds the smallest non-zero difference two sessions' medians can show. A cross-session spread
/// at or below it would be quantisation rather than stability — but see the module doc's CORRECTION
/// section: at S5's tighter `quantum <= 128` this bound is `<= 32 ns`, so the 512 ns spread these
/// sessions show is at least sixteen lattice steps of REAL variation, and the "one lattice step"
/// reading this constant once supported was an artifact of a loose bound.
const SV0_S1_5_MEDIAN_LATTICE_MAX_NS: f64 = 256.0;

/// How many DISTINCT tick values [`SV0_S1_5_QUANTUM_MAX_NS`] rests on — `None` because these
/// sessions ran the Rev-6 harness, which did not print `distinct_ticks`.
///
/// `None` is not a gap to be filled with a guess. It means "this bound has no recorded evidence",
/// and [`sv0_s1_5_measurement_meets_its_gates`] treats that exactly like insufficient evidence:
/// the lattice may not widen the spread gate. That is why the correction needs no re-run to be
/// SAFE — the loose bound is inert, not flattering. Re-measuring this rung under the corrected
/// harness is what fills it in, and the module doc's runbook says how.
const SV0_S1_5_QUANTUM_DISTINCT_TICKS: Option<usize> = None;

/// **Cross-rung evidence about the same device property.** The tightest `quantum_max_ns` rung S5's
/// eight sessions produced on this machine, in nanoseconds.
///
/// Not an S1.5 measurement and deliberately not used as one: it does not replace
/// [`SV0_S1_5_QUANTUM_MAX_NS`], which stays what these sessions measured. It is recorded here
/// because it is what falsified this file's earlier equality claim, and
/// [`sv0_s1_5_lattice_bound_is_a_bound_not_an_equality`] asserts its DIRECTION — a "correction"
/// that loosened the bound would be the flattering direction and must fail loudly.
///
/// The session that produced it was the first of a fresh process set and is discarded as a
/// measurement of the TERM. It remains valid evidence about the INSTRUMENT: its durations are
/// integer tick counts on the same counter whatever the clocks were doing, and it is precisely its
/// wide range that made the tighter bound recoverable.
const SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS: f64 = 128.0;

/// **`SV0_DEFERRED_TERM_REFERENCE`** (§6 S1.5) — the Deferred cost, in nanoseconds, of the SDF
/// soft-shadow + contact-AO term on S1's fixture at 512², measured by this bench.
///
/// §7 clause 3 adjudicates S5's own median paired delta against `2×` this number. Read it
/// together with this file's module doc: it is the cost of BOTH `pc.lighting_flags`-gated arms,
/// so it OVER-states the `!own_pixel` arm SV0 mirrors by the fraction
/// [`sv0_s1_5_confound_set_is_bounded`] measures.
///
/// Not independently typed: [`sv0_s1_5_measurement_meets_its_gates`] asserts it equals the median
/// of [`SV0_S1_5_SESSION_MEDIAN_DELTA_NS`], so it cannot drift away from its own evidence.
const SV0_DEFERRED_TERM_REFERENCE_NS: f64 = 6144.0;

/// S1's oracle records this many raster-covered mesh pixels on the fixture scene at 512²
/// (`sv0_scene`'s module doc: "the covered-mesh-pixel count is 28362 with the body present or
/// absent"). Asserted by [`sv0_s1_5_confound_set_is_bounded`] against THIS file's own copies of
/// the projection + coverage helpers, so a silent divergence between the two cannot leave the
/// confound ratio quantified over a raster the S1 gates never saw.
const SV0_SCENE_COVERED_MESH_PIXELS: usize = 28362;

/// The composite marcher's miss-distance bound — `sdf_gbuffer_composite.hlsl:441`'s `T_MAX`.
/// Declared locally for the same reason `sv0_oracle::MARCHER_EPS` is: the marcher's own budget
/// constants have no `compute.rs` export (its `EPS_COARSE`/`MAX_IT_COARSE` are the COARSE pass's
/// different pair).
const MARCHER_T_MAX: f32 = 10.0;

// ===========================================================================================
// The fixture
// ===========================================================================================

// ⚠️ **The `#[ignore]`d driver `sv0_deferred_term_bench()` is DELETED (profiling rung 7 step 6c).**
// It booted the fixture and called `app.run()`, relying on the runner's S1.5 loop to print a summary
// and return. That loop is retired with the harness, so the test would have rendered FOREVER: an
// `#[ignore]`d hang nothing in CI would ever notice, and the second time this rung has produced one
// (`vg_occ_split_timing.rs`'s worker listed a knob that no longer exits). **A test whose exit
// condition lived in the code being deleted is deleted with it.**
//
// What survives below is everything that was ever device-free: the arithmetic gates over the
// transcribed numbers. They still assert what the protocol requires of the figures on record.


// ===========================================================================================
// The gates over the transcribed numbers
// ===========================================================================================

/// The pre-registered protocol constants are self-consistent — checked independently of any
/// measurement, so it stays a live assertion during the window where nothing has been measured.
#[test]
fn sv0_s1_5_protocol_constants_are_pre_registered() {
    assert_eq!(
        SV0_BENCH_MIN_PAIRS, 30,
        "the plan's pair floor is 30 (§6 S1.5); lowering it changes the protocol, not the code"
    );
    assert_eq!(
        SV0_BENCH_SESSIONS, 3,
        "the plan's session count is 3 (§6 S1.5); the cross-session spread is what this rung gates"
    );
    // The quadruple floor must IMPLY the plan's pair floor, never merely rename it: each
    // quadruple carries two paired deltas.
    const {
        assert!(
            2 * SV0_BENCH_MIN_QUADS >= SV0_BENCH_MIN_PAIRS,
            "the quadruple floor must imply the plan's pair floor (one quadruple = two pairs)"
        );
    }
    // The two threshold invariants are `const` blocks rather than runtime asserts: both operands
    // are compile-time constants, so a widened gate should fail the BUILD, not merely a test run
    // someone can forget to invoke. Const-eval panics carry no formatting, hence the static text.
    const {
        assert!(
            SV0_SESSION_SPREAD_MAX > 0.0 && SV0_SESSION_SPREAD_MAX <= 0.10,
            "the spread gate is 10% (plan §6 S1.5); it may be TIGHTENED on new evidence, never \
             widened"
        );
    }
    const {
        assert!(
            SV0_NULL_CONTROL_MAX_FRACTION > 0.0
                && SV0_NULL_CONTROL_MAX_FRACTION <= SV0_SESSION_SPREAD_MAX,
            "the null control's drift floor must not exceed the spread gate it is read against — \
             otherwise a 'green' cross-session spread could be entirely drift"
        );
    }
}

/// The transcribed measurement, handed to the gates as VALUES rather than read as constants.
///
/// The indirection is deliberate and load-bearing in two ways. It gives the "nothing has been run
/// yet" state ONE clear failure that names the runbook, instead of four tests failing on NaN
/// comparisons a reader has to reconstruct. And it keeps the gates' assertions out of
/// compile-time-constant shape: a `const`-folded `assert!` would have to move into a `const {}`
/// block to satisfy the linter, and a const-block assertion on the UNMEASURED sentinel would fail
/// the BUILD — turning "this rung has not run yet" from a red test into a broken workspace.
struct Measured {
    /// `timestamp_period_ns` — ns per GPU timestamp tick (the SCALE).
    period_ns: f64,
    /// `quantum_max_ns` — an UPPER BOUND on the counter's step, in ns.
    quantum_max_ns: f64,
    /// `median_lattice_max_ns` — an upper bound on the lattice the reported session median lands
    /// on.
    lattice_max_ns: f64,
    /// How many distinct tick values the bound rests on; `None` when the session set did not
    /// record it.
    lattice_distinct: Option<usize>,
    /// The MEDIAN of the three session medians — the rung's central value, and what
    /// [`SV0_DEFERRED_TERM_REFERENCE_NS`] must equal.
    central_ns: f64,
}

impl Measured {
    /// Whether the transcribed lattice BOUND is allowed to widen the spread gate.
    ///
    /// A GCD over a homogeneous sample is a multiple of the true step, and a coarser lattice
    /// widens `max(protocol, lattice_floor)`. That is the ONE direction in which a degenerate
    /// sample can flatter a result, so the widening is licensed by evidence rather than granted by
    /// default: the bound must rest on at least [`SV0_LATTICE_MIN_DISTINCT_TICKS`] distinct tick
    /// values, and an unrecorded count (`None`) is treated as no evidence at all.
    ///
    /// Refusing to widen can only make the gate STRICTER, so a false `false` here costs a re-run,
    /// never a wrong verdict.
    fn lattice_may_widen(&self) -> bool {
        self.lattice_distinct.is_some_and(|n| n >= SV0_LATTICE_MIN_DISTINCT_TICKS)
    }
}

/// Every transcribed literal is present; returns them for the gates to read.
///
/// # Panics
///
/// With the runbook, until the orchestrator has run the GPU sessions and transcribed their output.
fn measured() -> Measured {
    let measured = SV0_S1_5_SESSION_MEDIAN_DELTA_NS.iter().all(|m| m.is_finite())
        && SV0_S1_5_SESSION_ORDER_BIAS_NS.iter().all(|m| m.is_finite())
        && SV0_S1_5_NULL_MEDIAN_DELTA_NS.is_finite()
        && SV0_S1_5_NULL_ORDER_BIAS_NS.is_finite()
        && SV0_S1_5_TIMESTAMP_PERIOD_NS.is_finite()
        && SV0_S1_5_QUANTUM_MAX_NS.is_finite()
        && SV0_S1_5_MEDIAN_LATTICE_MAX_NS.is_finite()
        && SV0_DEFERRED_TERM_REFERENCE_NS.is_finite();
    assert!(
        measured,
        "VB-SV0 S1.5 NOT MEASURED YET (expected until the orchestrator runs the GPU sessions).\n\
         Run, in three SEPARATE processes (PowerShell):\n  \
           $env:BOYKO_DISABLE_VALIDATION='1'; $env:BOYKO_SV0_BENCH='1'\n  \
           cargo test -p boyko-app --test sv0_deferred_term_bench sv0_deferred_term_bench \
             -- --ignored --nocapture --test-threads=1\n\
         then ONCE more with $env:BOYKO_SV0_BENCH_NULL='1' added (the null control).\n\
         Each session prints a `VB-SV0-S1.5 mode=…` line and a `VB-SV0-S1.5 RESOLUTION:` line; \
         this file's module doc carries the field-to-constant transcription table."
    );

    let mut sorted = SV0_S1_5_SESSION_MEDIAN_DELTA_NS;
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("invariant: transcribed medians are finite"));
    // `SV0_S1_5_QUANTUM_DISTINCT_TICKS` is deliberately absent from the completeness check above:
    // `None` is a RECORDED state for this rung ("the Rev-6 harness did not print it"), not a
    // forgotten transcription, and it is already the most conservative possible value — it forbids
    // the widening outright.
    Measured {
        period_ns: SV0_S1_5_TIMESTAMP_PERIOD_NS,
        quantum_max_ns: SV0_S1_5_QUANTUM_MAX_NS,
        lattice_max_ns: SV0_S1_5_MEDIAN_LATTICE_MAX_NS,
        lattice_distinct: SV0_S1_5_QUANTUM_DISTINCT_TICKS,
        central_ns: sorted[SV0_BENCH_SESSIONS / 2],
    }
}

/// **The S1.5 gate.** Adjudicates the transcribed measurements against everything §6 S1.5 and §7
/// clause 5 require: the sessions exist, each cleared the quadruple floor, the reference is the
/// sessions' own median, the cross-session spread is within the EFFECTIVE gate, and the null
/// control is below its pre-registered fraction of the armed median.
///
/// # This test is RED until the measurement exists, and that is the point
///
/// S1.5 is advertised as able to KILL the stage, so an un-run S1.5 must not read as a green rung
/// — the same "reddens by default … it fails unless the rung does the work" discipline the plan
/// applies to its own S2 gate (g). The failure text names the exact commands to run and the exact
/// literals to transcribe, so the red state is a runbook rather than a puzzle.
#[test]
fn sv0_s1_5_measurement_meets_its_gates() {
    let m = measured();

    for (i, quads) in SV0_S1_5_SESSION_QUADS.iter().enumerate() {
        assert!(
            *quads >= SV0_BENCH_MIN_QUADS,
            "session {i} collected {quads} quadruples, below the protocol floor of \
             {SV0_BENCH_MIN_QUADS} (§6 S1.5's ≥{SV0_BENCH_MIN_PAIRS} pairs, applied to the \
             statistic's own sample size) — re-run that session, do not lower the floor"
        );
    }

    // The transcribed RESOLUTION line must be internally consistent, or the lattice the spread
    // gate is read against is not the one the device reported. `quantum_max_ns` is an INTEGER
    // number of ticks (it is a GCD of tick counts) scaled by the period, and
    // `median_lattice_max_ns` is `quantum_max_ns` over 2 or 4. Both are cheap to check and both
    // catch a mis-transcription that would otherwise quietly move a gate.
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
         RESOLUTION line was mis-transcribed (the quantum bound IS a tick GCD times the period)"
    );
    let lattice_ratio = m.quantum_max_ns / m.lattice_max_ns;
    assert!(
        (lattice_ratio - 2.0).abs() <= 1e-6 || (lattice_ratio - 4.0).abs() <= 1e-6,
        "median_lattice_max_ns must be quantum_max_ns / 2 (odd quadruple count) or / 4 (even), but \
         quantum/lattice = {lattice_ratio}; the RESOLUTION line was mis-transcribed"
    );

    let central = m.central_ns;
    assert!(
        (SV0_DEFERRED_TERM_REFERENCE_NS - central).abs() <= 0.05,
        "SV0_DEFERRED_TERM_REFERENCE_NS ({SV0_DEFERRED_TERM_REFERENCE_NS}) must BE the median of \
         the session medians ({central}) — it is a transcription, not an independent number"
    );
    assert!(
        central > 0.0,
        "the armed median paired delta is {central} ns, i.e. arming the term did not cost \
         measurable time. Either the A/B never reached the shader (check that the `mode=armed` \
         run's median_armed_ns and median_cleared_ns actually differ) or the instrument is \
         blind — a bench that cannot see the term it exists to measure cannot bound SV0's cost"
    );

    // The EFFECTIVE spread gate. A gate finer than the smallest non-zero difference the
    // instrument can print is unreadable: below the lattice, "spread" and "quantisation" are the
    // same number. So the gate is the COARSER of the pre-registered 10% and the measured lattice —
    // and, so this can never be a silent widening, `sv0_s1_5_instrument_resolves_its_signal`
    // asserts separately that the lattice term does NOT bind.
    //
    // NEW, from the S5 finding: the lattice is a BOUND whose tightness depends on how varied the
    // sample happened to be, so a degenerate sample can hand this `max()` a flattering number.
    // The widening is therefore licensed by EVIDENCE (`Measured::lattice_may_widen`) rather than
    // granted by default; without it the gate stays at the protocol value, which can only ever be
    // stricter.
    let mut sorted = SV0_S1_5_SESSION_MEDIAN_DELTA_NS;
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("invariant: transcribed medians are finite"));
    let spread = (sorted[SV0_BENCH_SESSIONS - 1] - sorted[0]) / central;
    let lattice_floor = m.lattice_max_ns / central.abs();
    let may_widen = m.lattice_may_widen();
    let effective_max =
        if may_widen { SV0_SESSION_SPREAD_MAX.max(lattice_floor) } else { SV0_SESSION_SPREAD_MAX };
    let bound_by = if !may_widen {
        "the pre-registered 10% protocol gate (the lattice bound may NOT widen it: it rests on too \
         few distinct tick values, or on none that were recorded)"
    } else if lattice_floor > SV0_SESSION_SPREAD_MAX {
        "the instrument's measured quantisation lattice bound"
    } else {
        "the pre-registered 10% protocol gate"
    };
    println!(
        "VB-SV0-S1.5 gate: spread={spread:.4} effective_max={effective_max:.4} \
         (protocol={SV0_SESSION_SPREAD_MAX}, lattice_floor<={lattice_floor:.4}, \
         lattice_evidence={:?}, may_widen={may_widen}) bound by {bound_by}",
        m.lattice_distinct
    );
    assert!(
        spread <= effective_max,
        "S1.5 RED: cross-session spread {spread:.4} exceeds the effective gate {effective_max:.4} \
         (protocol {SV0_SESSION_SPREAD_MAX}, measured lattice floor <= {lattice_floor:.4}, \
         allowed to widen the gate: {may_widen}) over medians \
         {SV0_S1_5_SESSION_MEDIAN_DELTA_NS:?}. The instrument is not trustworthy at this \
         scale, so §7's cost clause cannot be adjudicated — this is §7 clause 5's defined \
         outcome (an owner VALUES call: revert, or ship unmeasured with the spread recorded and \
         clause 3 explicitly waived), NOT a licence to widen the gate. If may_widen is false, the \
         lattice term was REFUSED because its bound rests on fewer than \
         {SV0_LATTICE_MIN_DISTINCT_TICKS} distinct tick values (or on an unrecorded count) — \
         re-measure with the corrected harness rather than reasoning about what the lattice \
         'probably' is"
    );

    let null_budget = SV0_NULL_CONTROL_MAX_FRACTION * central;
    assert!(
        SV0_S1_5_NULL_MEDIAN_DELTA_NS.abs() <= null_budget,
        "S1.5 NULL CONTROL FAILED: two IDENTICAL configurations produced a median paired delta of \
         {SV0_S1_5_NULL_MEDIAN_DELTA_NS} ns, above the pre-registered budget of {null_budget} ns \
         ({SV0_NULL_CONTROL_MAX_FRACTION} × {central}). Under the counterbalanced design the \
         constant ordering bias is cancelled by construction, so a residual this large is NOT the \
         Revision 1 failure repeating — it is a SECOND-order position effect (or a drop pattern \
         that is quietly correlated with the cycle), and no number this harness produced means \
         anything until it is explained. Read median_order_bias_ns beside it: a large bias with a \
         small null is the design working; a large null is the design failing"
    );
}

/// **The resolution disclosure** — asserts that the measured quantisation lattice does NOT bind
/// the spread gate, i.e. that this instrument can resolve the signal finer than the protocol
/// tolerance it is judged at.
///
/// # Why this is a separate, non-waivable test
///
/// [`sv0_s1_5_measurement_meets_its_gates`] compares the spread against
/// `max(protocol, lattice_floor)`, because a gate finer than the instrument's own resolution
/// cannot be read. That `max` is correct AND it is exactly the shape a failing run could hide
/// inside: raise the lattice and the gate widens itself. So the lattice term is asserted here, on
/// its own, where widening it is not an option.
///
/// A failure here is NOT a code defect and NOT a bug to fix by editing this file. It is the
/// honest statement "this instrument cannot resolve better than ±X% at this signal size", which is
/// precisely §7 clause 5's defined outcome — a legitimate result, requiring an owner VALUES call
/// rather than a repair. The remedies, in order of preference:
///
/// 1. More quadruples (`BOYKO_SV0_BENCH_QUADS`). This does NOT move the lattice — a median of
///    lattice-valued samples is lattice-valued however many you take — so it helps only if the
///    failure is marginal and driven by an odd/even quadruple count. Cheap, so try it first.
/// 2. Raise the signal by changing the measured scene (more SDF bodies, a larger render extent,
///    more marcher work per pixel). This DOES move the ratio, and it costs the rung its meaning:
///    the number would no longer describe the scene S1's oracle certified, and §7 clause 3's `2×`
///    comparison would force S5 onto the same altered scene. Only with the owner's agreement, and
///    only applied to S1.5 and S5 together.
/// 3. Waive clause 3 under §7 clause 5, recording this test's numbers as the reason.
#[test]
fn sv0_s1_5_instrument_resolves_its_signal() {
    let m = measured();
    let central = m.central_ns;
    assert!(
        central > 0.0,
        "the armed median is {central} ns; resolution cannot be judged against a non-positive \
         signal (see sv0_s1_5_measurement_meets_its_gates for what that means)"
    );
    let lattice_floor = m.lattice_max_ns / central;
    println!(
        "VB-SV0-S1.5 resolution: quantum<={} ns median_lattice<={} ns signal_ns={central} \
         lattice_floor<={lattice_floor:.4} vs protocol {SV0_SESSION_SPREAD_MAX} \
         (evidence: {:?} distinct tick values)",
        m.quantum_max_ns, m.lattice_max_ns, m.lattice_distinct
    );
    assert!(
        lattice_floor <= SV0_SESSION_SPREAD_MAX,
        "S1.5 RESOLUTION-BOUND: this instrument cannot resolve better than ±{:.1}% at this signal \
         size ({} ns lattice bound on a {central} ns term), which is coarser than the \
         {SV0_SESSION_SPREAD_MAX} protocol gate. The cross-session spread therefore cannot \
         distinguish drift from quantisation, and §7's cost clause cannot be adjudicated on it. \
         This is §7 clause 5's defined outcome — a legitimate result, not a failure to code \
         around. See this test's doc for the three remedies; NONE of them is editing \
         SV0_SESSION_SPREAD_MAX or SV0_S1_5_MEDIAN_LATTICE_MAX_NS. Note the lattice is an UPPER \
         bound, so a failure here may also mean the bound is merely LOOSE — re-measuring with the \
         corrected harness, which prints distinct_ticks and min_tick_gap, distinguishes 'the \
         instrument is blunt' from 'this sample was homogeneous'",
        lattice_floor * 100.0,
        m.lattice_max_ns
    );
}

/// **The lattice figure is a BOUND, and this test is where that is stated in code.**
///
/// # The claim this file used to make, and why it was wrong
///
/// Revisions up to Rev 6 concluded "the counter's real step is 1024 ns", and read Revision 1's
/// 8.3% cross-session spread as "exactly one half-quantum — the gate measured its own resolution".
/// Both rest on treating a GCD over observed durations as a measurement of the hardware step. It is
/// not: for durations `t_i = m_i · G` the GCD returns `G · gcd(m_1 … m_n)`, which equals `G` only
/// when the observed multipliers are setwise coprime. A fixed-workload dispatch produces durations
/// clustered on a handful of values, and clustered multipliers routinely share a factor.
///
/// Rung S5 supplied the counter-example: eight sessions of the same protocol on the same device,
/// seven reporting `tick_gcd = 1024` and one — the one whose durations ranged widest — reporting
/// **128**. So `G <= 128`, this rung's 1024 was a bound eight times looser than necessary, and the
/// "one half-quantum" reading of the 8.3% spread was wrong: that spread was real session variation
/// across at least sixteen lattice steps.
///
/// # What is asserted here
///
/// The DIRECTION of the correction, which is the only way this arithmetic can be wrong and still
/// look right. A tighter bound makes every downstream gate STRICTER; a "correction" that loosened
/// it would widen the gate, which is the false-GREEN direction and exactly what a later reader
/// under pressure would be tempted to write. So the cross-rung figure must be strictly below this
/// rung's own, and both must be positive.
///
/// # What is only reported
///
/// The numbers themselves. There is no pass/fail threshold on a device property, and inventing one
/// would be the fitted-to-the-observation defect this campaign keeps finding one level down.
#[test]
fn sv0_s1_5_lattice_bound_is_a_bound_not_an_equality() {
    let m = measured();
    println!(
        "VB-SV0-S1.5 lattice bound: this rung's sessions support quantum <= {} ns (evidence: {:?} \
         distinct tick values). Rung S5's sessions, on the SAME device, support quantum <= {} ns — \
         {:.1}x tighter. The earlier 'the counter's real step is 1024 ns' was an artifact of \
         SAMPLE HOMOGENEITY, not a device measurement.",
        m.quantum_max_ns,
        m.lattice_distinct,
        SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS,
        m.quantum_max_ns / SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS
    );
    // The quadruple count is even on every session of this rung (200), so the reported median
    // lands on `quantum / 4`; deriving the corrected lattice rather than writing it down keeps it
    // tied to the one cross-rung literal above.
    let mut sorted = SV0_S1_5_SESSION_MEDIAN_DELTA_NS;
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("invariant: transcribed medians are finite"));
    let peak_to_peak = sorted[SV0_BENCH_SESSIONS - 1] - sorted[0];
    let corrected_lattice = SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS / 4.0;
    println!(
        "  Consequence for this file's own numbers: the median lattice is <= {corrected_lattice} \
         ns, not {} ns, so the {peak_to_peak} ns peak-to-peak these sessions show is >= {:.0} \
         lattice steps of REAL variation. The gate's verdict is unchanged (both readings sit under \
         the {} protocol tolerance, so the lattice term never bound), but 'the spread was \
         quantisation' is not available as an explanation of it.",
        m.lattice_max_ns,
        peak_to_peak / corrected_lattice,
        SV0_SESSION_SPREAD_MAX
    );

    // Compile-time: both operands are literals, and a non-positive bound would make every
    // resolution floor vanish — that must fail the BUILD, not a test run someone can forget to
    // invoke. Safe as a `const` block precisely because this constant is a recorded observation
    // and never the UNMEASURED sentinel (a const-block assertion on NaN would break the
    // workspace, which is why the MEASURED literals are asserted at runtime instead).
    // Const-eval panics carry no formatting, hence the static text.
    const {
        assert!(
            SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS > 0.0,
            "the cross-rung quantum bound must be positive; a non-positive one would make every \
             resolution floor vanish and every gate look trustworthy for the wrong reason"
        );
    }
    assert!(
        SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS < m.quantum_max_ns,
        "SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS ({SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS}) is not strictly \
         below this rung's own bound ({}), so it is not a correction at all. A cross-rung figure \
         may only ever TIGHTEN a bound: a looser one would widen every gate that reads the lattice, \
         which is the false-GREEN direction. If a re-measurement genuinely produced a coarser \
         bound, that belongs in SV0_S1_5_QUANTUM_MAX_NS as a re-measured literal — not here",
        m.quantum_max_ns
    );
    // The bound must divide into whole ticks, exactly like the rung's own — it is the same kind of
    // number (a tick GCD times the period), and a mis-transcription that broke that would be a
    // number from a different device or a different scale.
    let cross_ticks = SV0_S5_CROSS_RUNG_QUANTUM_MAX_NS / m.period_ns;
    assert!(
        (cross_ticks - cross_ticks.round()).abs() <= 1e-6 && cross_ticks.round() >= 1.0,
        "the cross-rung bound is {cross_ticks} ticks at this rung's {} ns period, which is not a \
         whole number — the two rungs did not read the same counter, or one figure was \
         mis-transcribed",
        m.period_ns
    );
}

/// **The ordering bias, read rather than merely cancelled.**
///
/// The counterbalanced design removes a constant ordering/ring-slot bias by algebra. That algebra
/// is only sound if the bias really is constant over a quadruple — an assumption, and one a
/// design that averages it away would leave permanently unexamined. So the harness estimates the
/// bias per quadruple and reports its median, and this test reads the four independent estimates
/// (three armed sessions plus the null control, since the position effect does not depend on the
/// A/B word) against each other.
///
/// # What is asserted, and what is only reported
///
/// REPORTED: the bias magnitude as a fraction of the signal — the contamination Revision 1's ABAB
/// deltas each carried, which is the number that justifies the redesign existing at all.
///
/// ASSERTED: sign agreement across the four runs, but ONLY when every estimate is larger than the
/// instrument's own lattice. Below the lattice the sign of an estimate is not a measurement, so
/// requiring agreement there would red the test on rounding. Above it, a sign flip means the
/// "ordering bias" is not a stable property of the harness at all — in which case cancelling it is
/// harmless but the real limitation is variance, and a reader must be told rather than reassured.
#[test]
fn sv0_s1_5_order_bias_is_reported() {
    let m = measured();
    let central = m.central_ns;

    let biases = [
        SV0_S1_5_SESSION_ORDER_BIAS_NS[0],
        SV0_S1_5_SESSION_ORDER_BIAS_NS[1],
        SV0_S1_5_SESSION_ORDER_BIAS_NS[2],
        SV0_S1_5_NULL_ORDER_BIAS_NS,
    ];
    println!(
        "VB-SV0-S1.5 order bias: armed={:?} null={SV0_S1_5_NULL_ORDER_BIAS_NS} \
         signal_ns={central} lattice_ns<={}",
        SV0_S1_5_SESSION_ORDER_BIAS_NS, m.lattice_max_ns
    );
    for (i, b) in biases.iter().enumerate() {
        println!(
            "  run {i}: bias {b} ns = {:.1}% of the signal — the amount a strict ABAB \
             alternation would have ADDED to every one of its deltas",
            100.0 * b / central
        );
    }

    // Sign agreement is only a claim about the world where every estimate is resolvable; below
    // the lattice it is a claim about rounding.
    //
    // ⚠️ The lattice is an UPPER bound, so this threshold is CONSERVATIVE in the direction of not
    // asserting: a loose bound can only suppress the assertion (calling a real sign "rounding"),
    // never fire it on noise. That weakens the test rather than falsifying it, which is the
    // acceptable direction here — but it is a second reason the bound's evidence matters, and it
    // is why `Measured::lattice_may_widen` exists rather than a blanket trust in the number.
    let resolvable = biases.iter().all(|b| b.abs() > m.lattice_max_ns);
    if resolvable {
        let positive = biases[0] > 0.0;
        assert!(
            biases.iter().all(|b| (*b > 0.0) == positive),
            "the ordering-bias estimates disagree in SIGN across runs ({biases:?}) while every \
             one of them is above the instrument's {} ns lattice. The counterbalance's premise is \
             that this bias is a stable offset over a quadruple; four resolvable estimates that \
             cannot agree on its direction do not support that premise. The cancellation is still \
             arithmetically harmless, but it is no longer the reason the numbers are trustworthy \
             — say so when reporting, and treat the null control as the only evidence that the \
             design works",
            m.lattice_max_ns
        );
    } else {
        println!(
            "  NOTE: at least one bias estimate is at or below the {} ns lattice BOUND, so sign \
             agreement is not asserted — an unresolvable estimate's sign is rounding, not \
             evidence. Because the lattice is an upper bound, a LOOSE one suppresses this \
             assertion more often than a tight one would; the corrected harness's distinct_ticks \
             is what says whether that happened. A bias this small also means the counterbalance \
             was not load-bearing on these runs, which is worth stating rather than assuming.",
            m.lattice_max_ns
        );
    }
}

// ===========================================================================================
// The confound bound — how much of the measured delta is NOT the `!own_pixel` arm
// ===========================================================================================

/// Component-wise difference `a - b`. Local because `sv0_oracle`'s own vector helpers are private
/// to that module, and widening them would edit a file S1's shipped gates are quantified over for
/// the sake of four lines.
#[inline]
fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Dot product.
#[inline]
fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Euclidean length.
#[inline]
fn v_len(a: [f32; 3]) -> f32 {
    v_dot(a, a).sqrt()
}

/// Unit-normalizes `a`, returning the ZERO vector for a degenerate input rather than `NaN` — the
/// same zero-guard discipline `sv0_oracle::v_normalize` carries.
#[inline]
fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len_sq = v_dot(a, a);
    if len_sq <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let inv = len_sq.sqrt().recip();
    [a[0] * inv, a[1] * inv, a[2] * inv]
}

/// The fixtures' projection — `sv0_adequacy.rs::scene_view_proj_rows` verbatim.
///
/// Duplicated rather than shared because both live in `tests/` binaries that cannot import each
/// other; the duplication is made safe by [`sv0_s1_5_confound_set_is_bounded`]'s assertion that
/// this file's coverage count equals the one S1's oracle recorded.
fn scene_view_proj_rows() -> [[f32; 4]; 4] {
    let view = ViewUniform::from_camera(
        sv0_scene::camera_transform().to_affine(),
        sv0_scene::camera_projection(),
    );
    boyko_render::forward_view_proj_rows(&view, sv0_scene::DUMP_EXTENT, sv0_scene::DUMP_EXTENT)
}

/// The fixtures' raster coverage — `sv0_adequacy.rs::scene_coverage` verbatim (see
/// [`scene_view_proj_rows`] for why it is duplicated and what makes that safe).
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

/// The world-space point whose projection is pixel-centre `(px, py)` at clip `w == depth`.
///
/// # Why this INVERTS the projection rows instead of rebuilding a camera basis
///
/// The confound count has to be quantified over the SAME screen mapping the oracle's rasterizer
/// uses, and that mapping lives in `view_proj_rows` — the y-flip, the aspect, the `[-1,1] →
/// [0,extent]` convention and all. Reconstructing an eye/forward/right/up basis from
/// `sv0_scene::camera_transform()` would re-derive those conventions by hand, and a sign error
/// there would silently shift the whole count. Solving the rows directly cannot: it is the
/// rasterizer's own arithmetic run backwards.
///
/// Three of the four rows suffice, because `row 2` (depth) is not part of the screen mapping:
///
/// ```text
/// row_x · (P,1) = ndc_x · w
/// row_y · (P,1) = ndc_y · w
/// row_w · (P,1) = w
/// ```
///
/// a 3×3 linear system in `P`, solved by Cramer's rule.
///
/// # Panics
///
/// Panics on a singular system — that would mean the fixture's projection rows are degenerate,
/// a construction error rather than a runtime condition.
fn unproject(rows: [[f32; 4]; 4], px: u32, py: u32, extent: u32, depth: f32) -> [f32; 3] {
    let fe = extent as f32;
    let ndc_x = ((px as f32 + 0.5) / fe) * 2.0 - 1.0;
    let ndc_y = ((py as f32 + 0.5) / fe) * 2.0 - 1.0;

    let m = [
        [rows[0][0], rows[0][1], rows[0][2]],
        [rows[1][0], rows[1][1], rows[1][2]],
        [rows[3][0], rows[3][1], rows[3][2]],
    ];
    let b = [
        ndc_x * depth - rows[0][3],
        ndc_y * depth - rows[1][3],
        depth - rows[3][3],
    ];

    let det3 = |c: [[f32; 3]; 3]| {
        c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1])
            - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
            + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0])
    };
    let det = det3(m);
    assert!(det.abs() > 1e-12, "invariant: the fixture's projection rows are non-singular");

    let mut out = [0.0f32; 3];
    for col in 0..3 {
        let mut mc = m;
        for row in 0..3 {
            mc[row][col] = b[row];
        }
        out[col] = det3(mc) / det;
    }
    out
}

/// The exact ray-sphere entry parameter along a UNIT direction, or `None` on a miss / a hit
/// behind the eye.
///
/// Analytic rather than sphere-traced: the fixture's edit list is ONE `SdfEdit::sphere` with zero
/// smoothing, so the field is exactly `‖p − C‖ − r` and the analytic root is what a converged
/// trace approaches. [`sv0_s1_5_confound_set_is_bounded`] pins that identification against the
/// shipped field (`sv0_oracle::field_distance`) before using it.
fn ray_sphere_t(origin: [f32; 3], dir: [f32; 3], center: [f32; 3], radius: f32) -> Option<f32> {
    let oc = v_sub(origin, center);
    let b = v_dot(dir, oc);
    let c = v_dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t_near = -b - root;
    let t = if t_near > 0.0 { t_near } else { -b + root };
    (t > 0.0).then_some(t)
}

/// **Quantifies the confound this file's module doc names**: how many pixels run the `own_pixel`
/// SDF-hit arm (`sdf_gbuffer_composite.hlsl:1805`) that the A/B switches on alongside the
/// `!own_pixel` arm (`:1865`) SV0 actually mirrors.
///
/// Reports the ratio, which is the factor by which `SV0_DEFERRED_TERM_REFERENCE_NS` over-states
/// the term SV0 would inline — the false-GREEN direction for §7 clause 3, hence measured rather
/// than assumed.
///
/// # What is asserted, and what is only reported
///
/// ASSERTED: the instrument's own soundness — that this file's projection reproduces the S1
/// oracle's raster exactly, that its ray generation agrees with that raster's own world
/// positions, and that its analytic sphere IS the shipped field. Those are the three ways the
/// ratio could be a quiet lie.
///
/// REPORTED, not gated: the ratio itself. There is no pre-registered threshold for it because it
/// is not a pass/fail property of the fixture — it is a bias in the reference that the
/// orchestrator applies when adjudicating §7 clause 3. Inventing a floor for it here would be the
/// fitted-to-the-observation defect S1's own floor derivation exists to avoid.
///
/// # The one way the count can be wrong
///
/// The shipped marcher is DISCRETE (`EPS` / `MAX_IT` / over-relaxation), so at the silhouette it
/// can classify a grazing pixel differently from this analytic root. That is a boundary-pixel
/// disagreement on a ~200-pixel perimeter against a ~3000-pixel disc, i.e. well under the
/// precision anything downstream reads this ratio at.
#[test]
fn sv0_s1_5_confound_set_is_bounded() {
    let coverage = scene_coverage();
    assert_eq!(
        coverage.near_rejected_triangles, 0,
        "the fixture's raster must not silently drop near-plane triangles"
    );
    assert_eq!(
        coverage.covered_count(),
        SV0_SCENE_COVERED_MESH_PIXELS,
        "this file's duplicated projection/coverage helpers no longer reproduce the raster S1's \
         gates are quantified over — the confound ratio below would be measured against a \
         different scene than the reference it corrects"
    );

    let rows = scene_view_proj_rows();
    let eye = sv0_scene::CAMERA_EYE;
    let extent = sv0_scene::DUMP_EXTENT;
    let center = sv0_scene::sdf_body_center();
    let radius = sv0_scene::SDF_SPHERE_RADIUS;

    // (1) The analytic sphere IS the shipped field — checked at the surface, where a radius or
    // centre error shows up at full size rather than being absorbed into a distance.
    let edits = [sv0_scene::sdf_body_edit()];
    let on_surface = [center[0] + radius, center[1], center[2]];
    let field_at_surface = sv0_oracle::field_distance(&edits, on_surface);
    assert!(
        field_at_surface.abs() < 1e-4,
        "the analytic sphere disagrees with the shipped field at its own surface \
         (field = {field_at_surface}); the ray-sphere shortcut below is only valid because the \
         fixture's single un-smoothed sphere edit makes the two identical"
    );

    // (2) The ray generation agrees with the rasterizer's own world positions. Any convention
    // error (y-flip, half-pixel, aspect) shows up here as an angular disagreement, before it can
    // shift the count.
    let mut worst_cos = 1.0f32;
    for y in 0..extent {
        for x in 0..extent {
            let Some(pixel) = coverage.get(x, y) else { continue };
            let p = unproject(rows, x, y, extent, 5.0);
            let dir = v_normalize(v_sub(p, eye));
            let truth = v_normalize(v_sub(pixel.world_pos, eye));
            worst_cos = worst_cos.min(v_dot(dir, truth));
        }
    }
    assert!(
        worst_cos > 1.0 - 1e-4,
        "the unprojected eye rays disagree with the rasterizer's own covered-pixel directions \
         (worst cos = {worst_cos}); the confound count would be quantified over the wrong pixels"
    );

    // (3) The confound set: pixels where the SDF surface is hit in FRONT of the mesh, i.e. where
    // the `own_pixel` arm's two marches run. `t_mesh` is the eye→surface distance for a covered
    // pixel; an uncovered pixel has no mesh, so the marcher's own miss bound is the only ceiling.
    let mut sdf_visible = 0usize;
    for y in 0..extent {
        for x in 0..extent {
            let p = unproject(rows, x, y, extent, 5.0);
            let dir = v_normalize(v_sub(p, eye));
            let Some(t) = ray_sphere_t(eye, dir, center, radius) else { continue };
            if t > MARCHER_T_MAX {
                continue;
            }
            let t_mesh = match coverage.get(x, y) {
                Some(pixel) => v_len(v_sub(pixel.world_pos, eye)),
                None => MARCHER_T_MAX,
            };
            if t < t_mesh {
                sdf_visible += 1;
            }
        }
    }

    let covered = coverage.covered_count();
    let ratio = sdf_visible as f64 / covered as f64;
    println!(
        "VB-SV0-S1.5 confound: sdf_visible_px={sdf_visible} covered_mesh_px={covered} \
         ratio={ratio:.4}"
    );
    println!(
        "  The A/B's median paired delta covers BOTH lighting_flags-gated arms, so \
         SV0_DEFERRED_TERM_REFERENCE_NS over-states the `!own_pixel` arm SV0 mirrors. Deflate by \
         roughly 1/(1+ratio) = {:.4} before adjudicating plan §7 clause 3, and treat that as an \
         upper bound on the correction (the two arms' PER-PIXEL costs are not asserted equal).",
        1.0 / (1.0 + ratio)
    );

    assert!(
        sdf_visible > 0,
        "the SDF body is not visible at all on this fixture, which contradicts sv0_scene's own \
         placement derivation (a ~66 px disc) — the ray generation or the placement has moved"
    );
}
