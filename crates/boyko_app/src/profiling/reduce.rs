//! Profiling rung 7 — the window reducer: retired GPU pairs in, [`ZoneRow`]s out.
//!
//! [`super::artifact`] is the file; this is what fills it. It takes the `PairResult` slices the
//! `GpuZoneRecorder` hands its retire sink, accumulates per zone across a window of frames, and
//! reduces to the median / mean / p95 / offsets the artifact carries.
//!
//! # It has no console form, and that is the point
//!
//! The corpus states the reducer *"has no console form at all. Every value it produces goes into
//! the TOML artifact or the binary stream"*, and says why: **that is what lets rung 7 delete the
//! stdout measurement channel.** A reducer with a `print` would leave the channel alive under
//! another name.
//!
//! # THE STATISTICS ARE THE SHIPPED ONES, DELIBERATELY
//!
//! [`stats_ns`] is `runner.rs`'s `vb_bench_stats_ns`, conventions and all: mean over the raw
//! samples; median as the average of the two central values on an even count; p95 as
//! `sorted[(n * 0.95) as usize]` clamped to the last index. That is not laziness — rung 7a's whole
//! justification for writing figures at one decimal was that they stay **directly comparable with
//! the printed lines**, and a different median convention makes them incomparable for a reason
//! that has nothing to do with the channel. When rung 8 changes a convention it changes it for
//! both, or the comparison it licenses is between two instruments rather than two channels.
//!
//! # Offsets are per FRAME before they are reduced
//!
//! A zone's begin offset is measured from **its own frame's** base — the earliest measured begin in
//! that frame — and only then folded across frames. Reducing the raw `begin_ticks` instead would
//! reduce the GPU clock itself, which drifts across a window and means nothing. The end offset is
//! formed per frame from that frame's two halves for the same reason `runner.rs` states at its own
//! fold: *"adding the two published MEDIANS afterwards is not a time any frame had"*.
//!
//! # Allocation
//!
//! The per-zone sample vectors are `Vec`s, allocated once per zone at first sight and grown to the
//! window's length. This is off-frame, bench-only host code that runs after a frame's results are
//! read — not the recorder and not a hot path — and the alternative, a fixed `[f64; WINDOW]` per
//! possible zone, is 128 zones × 121 frames of `f64` reserved to hold a handful of rows.

use boyko_rhi_vulkan::present::gpu_zone::{
    GpuLabel, PairResult, ZONE_VB_GEO, ZONE_VB_PRESHADE, ZONE_VB_PRODUCE_NET, ZONE_VB_PRODUCE_RUN,
    ZONE_VB_SDF_MESH,
};
// `ZONE_VB_SHADE` is deliberately NOT imported here: after the C1 correction no declaration in
// this module names it, and an import kept "for symmetry" would be the first step back to putting
// a `TOP`-stamped id in a `BOTTOM` chain. The tests import it themselves, to assert its ABSENCE.

use super::artifact::{LabelCensus, OrderCensus, ZoneLabel, ZoneRow};

/// One zone's samples across the window.
struct ZoneAccum {
    /// The zone id, `family base + pass slot`.
    zone: u16,
    /// The WORST label this zone showed in any frame of the window.
    ///
    /// Worst rather than last: a window in which one frame tore is a window whose numbers are
    /// suspect, and a reader that saw only the final frame's label would be told otherwise.
    worst: ZoneLabel,
    /// Measured durations, ns.
    dur_ns: Vec<f64>,
    /// Measured begin offsets from each frame's own base, ns.
    begin_off_ns: Vec<f64>,
    /// Measured end offsets, formed per frame.
    end_off_ns: Vec<f64>,
}

/// How bad a label is. `Measured` is best; anything else means the numbers are not measurements.
fn severity(l: ZoneLabel) -> u8 {
    match l {
        ZoneLabel::Measured => 0,
        ZoneLabel::NotBracketed => 1,
        ZoneLabel::Lost => 2,
        ZoneLabel::Torn => 3,
    }
}

/// Maps the recorder's label onto the artifact's. Two enums because they belong to two crates and
/// one of them is the file format; the mapping is total and lives here alone.
fn label_of(l: GpuLabel) -> ZoneLabel {
    match l {
        GpuLabel::Measured => ZoneLabel::Measured,
        GpuLabel::NotBracketed => ZoneLabel::NotBracketed,
        GpuLabel::Lost => ZoneLabel::Lost,
        GpuLabel::Torn => ZoneLabel::Torn,
    }
}

/// What a chain member's ABSENCE from a frame's slice means on this leg — VB-SV0 DP6-0b.
///
/// # It keys on ABSENCE, and it is resolved against the leg, not against the label
///
/// A zone the recorder never opened has **no `PairResult` in the slice at all**: `alloc_pair` is
/// called only from `begin`, and the retire path iterates the pairs that were allocated. It is
/// ABSENT, not [`ZoneLabel::NotBracketed`] — so a policy keyed on the label would be keyed on
/// something the per-frame path does not deliver for the structural case.
///
/// And the two absences mean opposite things. A `ZONE_VB_PRESHADE` missing from a FUSED leg is
/// structural: the bracket is not in the command stream and its true contribution is zero. The
/// same zone missing from a SPLIT leg means `alloc_pair` refused on a full ring — the 256 µs of
/// work **executed** and only its bracket is gone. Contributing `0.0` there yields a ~4.5×
/// inflated derived sample, and the `n` floor cannot catch it because the sample was PUSHED rather
/// than skipped.
/// # TWO states, not three — and the third was DELETED rather than shipped unreachable
///
/// `docs/VB-SV0-DP6-DESIGN.md` §R4.3.4 specifies a third, `Optional(OptionalAbsence)`, for a member
/// that may or may not stamp on a leg. **No leg in this tree has one**: every zone the two shipped
/// specs name either always stamps on that leg or structurally cannot, so the variant and its
/// payload enum were written, found to have no constructor, and removed. Unreachable vocabulary in
/// a policy type is the dead-datum shape this rung exists to close, not a spare part — a claim that
/// three cases are handled, with one of them never having been executed.
///
/// The day a leg genuinely needs it, the variant returns **in the same commit as its consumer**,
/// which is the registry's own `Pending`-row discipline applied to an enum.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expect {
    /// The member cannot stamp on this leg. Absence is structural ⇒ it contributes `0.0`.
    Forbidden,
    /// The member must stamp on this leg. Absence is a runtime failure ⇒ **skip the frame**.
    Required,
}

/// One derived row: a difference of two brackets, formed AT THE FRAME — VB-SV0 DP6-0b.
///
/// # Formed per frame, then reduced — never the other way round
///
/// `03-STATISTICS.md`'s rule is `median_f(Σ_members)`, never `Σ_members(median_f)`, and the
/// campaign has measured the two disagreeing by 144–240 ns on a real reading. A derived row built
/// from two published medians would be exactly the forbidden composition; this one pushes ONE
/// sample per frame into an ordinary [`ZoneAccum`], so the artifact's reduction of it is the same
/// reduction every other row gets.
///
/// **Nothing is ever zipped.** The value is read out of the frame's own slice by zone id, so a
/// positional misalignment across `Lost` / absent frames cannot arise.
#[derive(Clone, Copy, Debug)]
pub struct DerivedSpec {
    /// The zone id the derived row is PUBLISHED under. Never stamped by any recorder.
    pub zone: u16,
    /// The containing bracket.
    pub minuend: u16,
    /// The bracket subtracted from it.
    pub subtrahend: u16,
    /// What [`Self::minuend`]'s absence means on this leg. Practically always
    /// [`Expect::Required`]: a derived row whose containing bracket did not run has no subject.
    pub minuend_expect: Expect,
    /// What [`Self::subtrahend`]'s absence means on this leg.
    pub subtrahend_expect: Expect,
}

/// The VB record-order chain on a **fused** (`!path_vb_split()`) mesh-leg boot — VB-SV0 DP6-0b.
///
/// **The FIRST element is the containing run**; every later member must both follow its
/// predecessor's BEGIN and nest inside the run.
///
/// Members: **12** (the run) and **10**. See [`VB_CHAIN_SPLIT`] for why the membership rule is
/// "restamped to `BOTTOM` by DP6-0b" and not "recorded inside the run".
pub static VB_CHAIN_FUSED: &[u16] = &[ZONE_VB_PRODUCE_RUN, ZONE_VB_SDF_MESH];

/// The VB record-order chain on a **split** (`path_vb_split()`) boot — VB-SV0 DP6-0b.
///
/// Members: **12** (the run), **10**, **11**, **13** — exactly the quartet `zone_begin_stage`
/// restamps to `BOTTOM_OF_PIPE`, in record order `12b … 10 … 11 … 13 … 12e`. Each is a
/// prefix-COMPLETION time, so consecutive stamps with no executable work between them partition
/// the span they divide, and a per-frame `begin(a) <= begin(b)` over them is a statement about one
/// monotone quantity.
///
/// # `ZONE_VB_SHADE` (id 2) is EXCLUDED, by name and for a stated reason
///
/// Id 2 keeps `TOP_OF_PIPE` — DP6-0b deliberately does not restamp it, so VB-P1d's published
/// break-even keeps its meaning, and the split producer's cost is obtained by DERIVATION
/// (`PRODUCE_RUN.end − PRESHADE.end`) instead. A `TOP` begin retires when the command is FETCHED,
/// not when the prefix completes, so `b2` can retire **before** `b10`/`b13` on the same frame
/// purely by command-processor fetch timing. Ordering it against `BOTTOM` begins would therefore
/// manufacture violations that describe the stage difference and nothing else —
/// non-deterministically, which is the shape this census exists to catch rather than to emit.
/// `gpu_zone.rs`'s `zone_begin_stage` states the general form: *"the `TOP` rows are NOT addable to
/// the `BOTTOM` rows as a partition"*, and the design's own tier table puts id 2 in the tier with
/// no per-frame check.
///
/// The two ids the design's tier table also leaves out are out for their own reasons: **6**, whose
/// POSITION is leg-dependent (see [`WindowReducer::check_order`]'s doc), and **14**, which is
/// derived and never stamped.
pub static VB_CHAIN_SPLIT: &[u16] =
    &[ZONE_VB_PRODUCE_RUN, ZONE_VB_SDF_MESH, ZONE_VB_GEO, ZONE_VB_PRESHADE];

/// `ZONE_VB_PRODUCE_NET` on a fused leg: `PRESHADE` is structurally absent, so `NET ≡ PRODUCE_RUN`.
///
/// The two rows use ONE comparator, which is the whole point of deriving it: G-NEUTRAL and
/// G-REDUCE read the same quantity on both sides of the fused/split discontinuity.
pub static VB_DERIVED_FUSED: &[DerivedSpec] = &[DerivedSpec {
    zone: ZONE_VB_PRODUCE_NET,
    minuend: ZONE_VB_PRODUCE_RUN,
    subtrahend: ZONE_VB_PRESHADE,
    minuend_expect: Expect::Required,
    subtrahend_expect: Expect::Forbidden,
}];

/// `ZONE_VB_PRODUCE_NET` on a split leg: `PRESHADE` must stamp, so its absence skips the frame.
pub static VB_DERIVED_SPLIT: &[DerivedSpec] = &[DerivedSpec {
    zone: ZONE_VB_PRODUCE_NET,
    minuend: ZONE_VB_PRODUCE_RUN,
    subtrahend: ZONE_VB_PRESHADE,
    minuend_expect: Expect::Required,
    subtrahend_expect: Expect::Required,
}];

/// What one term of a derived row contributed on one frame.
enum Term {
    /// Its measured duration, in GPU TICKS (scaled by the period once, at the difference).
    Value(f64),
    /// Structurally absent on this leg — its true contribution is zero.
    Zero,
    /// The frame cannot form this row.
    Skip,
}

/// The frame's `Measured` pair for `zone`, if it has one.
fn measured(pairs: &[PairResult], zone: u16) -> Option<&PairResult> {
    pairs.iter().find(|p| p.zone == zone && matches!(p.label, GpuLabel::Measured))
}

/// Resolves one term of a derived row against the leg's declared expectation.
///
/// A pair that is PRESENT but not `Measured` is never `Zero`, whatever the expectation says: the
/// recorder opened that bracket, so the work ran and only its number is missing. Contributing zero
/// there is the inflated-sample defect the expectation table exists to prevent.
fn resolve_term(pairs: &[PairResult], zone: u16, expect: Expect) -> Term {
    match pairs.iter().find(|p| p.zone == zone) {
        Some(p) if matches!(p.label, GpuLabel::Measured) => Term::Value(p.dur_ticks as f64),
        Some(_) => Term::Skip,
        None => match expect {
            Expect::Forbidden => Term::Zero,
            Expect::Required => Term::Skip,
        },
    }
}

/// Accumulates a window of retired frames and reduces it to the artifact's rows.
pub struct WindowReducer {
    zones: Vec<ZoneAccum>,
    census: LabelCensus,
    /// Nanoseconds per GPU tick (`VkPhysicalDeviceLimits::timestampPeriod`).
    period_ns: f64,
    /// Frames folded in, whatever they contained.
    frames: u32,
    /// The declared record-order chain — see [`VB_CHAIN_SPLIT`]. Empty means "this family declares
    /// no chain", which leaves `frames_checked` at zero and is therefore DISTINGUISHABLE from a
    /// chain that was checked and held.
    chain: &'static [u16],
    /// The declared derived rows — see [`VB_DERIVED_SPLIT`].
    derived: &'static [DerivedSpec],
    /// The per-frame verdicts, counted.
    order: OrderCensus,
}

impl WindowReducer {
    /// A reducer for a device whose timestamps advance `period_ns` per tick.
    ///
    /// `chain` and `derived` are DECLARATIONS, and they are parameters rather than a lookup so
    /// that a family shipping neither is a compile-site omission at the call rather than a silent
    /// pass: `&[]` is a legal argument and it publishes `frames_checked == 0`, which every gate
    /// reads as INCONCLUSIVE and none reads as a pass.
    #[must_use]
    pub fn new(
        period_ns: f64,
        chain: &'static [u16],
        derived: &'static [DerivedSpec],
    ) -> WindowReducer {
        WindowReducer {
            zones: Vec::new(),
            census: LabelCensus::default(),
            period_ns,
            frames: 0,
            chain,
            derived,
            order: OrderCensus::default(),
        }
    }

    /// Frames folded in so far.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The per-frame record-order verdicts so far.
    #[must_use]
    pub fn order(&self) -> &OrderCensus {
        &self.order
    }

    /// Folds one retired frame's pairs.
    pub fn observe_frame(&mut self, pairs: &[PairResult]) {
        self.frames += 1;
        // VB-SV0 DP6-0b: BOTH per-frame deliverables run here, before the accumulation below,
        // because this is the ONLY place in the tree that holds one frame's `PairResult` slice.
        // Written as a test over the artifact instead, a containment check would compare window
        // MEDIANS — a different statement, and `vg_occ_split_timing.rs:669-672` records that exact
        // composition reporting an inequality backwards by 144 ns. A `TOP`-stamped member violates
        // the chain non-deterministically, which is precisely what a median averages away.
        self.check_order(pairs);
        // THIS frame's base: the earliest measured begin in it. A frame with no measured pair has
        // no base and contributes no offsets — which is different from contributing an offset of
        // zero, and the difference is the whole reason the label travels beside the numbers.
        let base = pairs
            .iter()
            .filter(|p| matches!(p.label, GpuLabel::Measured))
            .map(|p| p.begin_ticks)
            .min();
        self.fold_derived(pairs, base);

        for p in pairs {
            // VB-SV0 DP6-0b: a slice pair carrying a DECLARED DERIVED id must never reach the
            // accumulator. Nothing stamps id 14 today and a source pin says so, but the failure
            // this guards is silent and unrecoverable if it ever does: the pair's duration would be
            // pushed into the SAME `ZoneAccum` the derived value is pushed into, and the published
            // row would be a reduction over a mixture of a bracket and a difference — arithmetic
            // over two unrelated quantities, under one key, with no label able to say so.
            //
            // SKIP, and count it as a torn-class loss rather than dropping it silently: a pair the
            // recorder really did open and this reducer really did refuse is a loss, and the census
            // is where losses are stated.
            if self.derived.iter().any(|d| d.zone == p.zone) {
                debug_assert!(
                    false,
                    "invariant: zone {} is a DERIVED id and must not be stamped by any recorder; \
                     a `PairResult` for it arrived from the frame's slice",
                    p.zone
                );
                self.census.torn += 1;
                Self::record_device_loss();
                continue;
            }
            let label = label_of(p.label);
            match label {
                ZoneLabel::Measured => self.census.measured += 1,
                ZoneLabel::NotBracketed => self.census.not_bracketed += 1,
                ZoneLabel::Lost => {
                    self.census.lost += 1;
                    Self::record_device_loss();
                }
                ZoneLabel::Torn => {
                    self.census.torn += 1;
                    Self::record_device_loss();
                }
            }

            let idx = self.accum_index(p.zone, label);
            let acc = &mut self.zones[idx];
            if severity(label) > severity(acc.worst) {
                acc.worst = label;
            }
            let (Some(base), ZoneLabel::Measured) = (base, label) else { continue };
            // `begin_ticks` is masked to the device's valid bits and monotone within a frame, so
            // the subtraction cannot underflow for the pair that IS the base or any pair after it.
            let begin_off = (p.begin_ticks.saturating_sub(base) as f64) * self.period_ns;
            let dur = (p.dur_ticks as f64) * self.period_ns;
            acc.dur_ns.push(dur);
            acc.begin_off_ns.push(begin_off);
            acc.end_off_ns.push(begin_off + dur);
        }
    }

    /// The frame's record-order verdict over the declared chain — VB-SV0 DP6-0b.
    ///
    /// # The output is a COUNT, not a statistic
    ///
    /// A median can average a violation away; a count cannot. With a per-frame violation
    /// probability `p` over a ~100-frame window, `P(0 violations) = (1-p)^100` — at the 2.5–8.3 %
    /// overlap the particle lane measured on this box, below `10⁻³`.
    ///
    /// # What it requires, per frame, on RAW ticks
    ///
    /// The chain's **first** element is the containing run. For every later member that is present
    /// and `Measured` this frame: its BEGIN is at or after the previous present member's BEGIN, and
    /// its whole interval nests inside the run's. **Ties are legal** — equal-tick `BOTTOM_OF_PIPE`
    /// stamps give equal offsets, and this box's empty-bracket floor is 96–128 ticks.
    ///
    /// An ABSENT member does not break the chain: the next present member is compared against the
    /// last present one. That is stronger than comparing only literally-consecutive pairs and it is
    /// still sound — the chain declares a total order, so `a ≤ c` must hold whether or not `b`
    /// stamped — and it is what keeps coverage when a leg-conditional member is missing.
    ///
    /// # Why `ZONE_VB_HZB_BUILD` (id 6) is NOT a member — the real reason, measured against the host
    ///
    /// The design asked for id 6 "leg-conditionally, driven by the expectation table", and the
    /// first version of this code left it out calling its position merely *leg*-dependent. That is
    /// half the answer and the weaker half. **Its position is not boot-frozen at all**:
    /// `path_vb_occlusion_split()` conjoins `scene.vb_occlusion.is_some()` — recomputed EVERY FRAME
    /// in `boyko_app::runner` from a LIVE `OcclusionConfig` resource (rung P4-4 turned that regime
    /// from a boot env read into a live resource) — and `scene.vb_occlusion_instances > 0`, which
    /// is **this frame's** count of instances carrying the `OcclusionCulling` marker.
    ///
    /// So id 6 can occupy its `ZONE_VB_RUN`-side slot on one frame of a window and its
    /// post-producer slot on the next, with **no boot-time predicate able to say which** — while
    /// `chain` is one `&'static [u16]` chosen once, at reducer construction. A declaration naming
    /// id 6 would therefore be right on some frames of a single window and wrong on others, and
    /// the wrong ones would be counted as violations of an order the recorder never promised.
    ///
    /// Its containment is still real and still stated — on a `!occlusion_split` frame it sits
    /// inside `ZONE_VB_PRODUCE_RUN` — and the gate reads it the way the design intends: by
    /// asserting the two sides of a comparison agree in occlusion-split arming, not by ordering it
    /// per frame.
    ///
    /// At most ONE violation is counted per member per frame, at its worst delta: three deltas of
    /// one displaced bracket are one defect, and counting three would make the census a measure of
    /// how many rules a single fault happens to touch.
    fn check_order(&mut self, pairs: &[PairResult]) {
        let Some((&run_id, members)) = self.chain.split_first() else {
            // No declared chain: `frames_checked` stays 0, which every gate reads as INCONCLUSIVE.
            return;
        };
        let Some(run) = measured(pairs, run_id) else { return };
        self.order.frames_checked += 1;
        let run_begin = run.begin_ticks;
        let run_end = run.begin_ticks + run.dur_ticks;
        let mut prev_begin = run_begin;
        for &m in members {
            let Some(p) = measured(pairs, m) else { continue };
            let end = p.begin_ticks + p.dur_ticks;
            // Each term is 0 exactly when its rule holds, so the max is the frame's worst breach of
            // this member and `> 0` is the verdict.
            let out_of_order = prev_begin.saturating_sub(p.begin_ticks);
            let before_run = run_begin.saturating_sub(p.begin_ticks);
            let past_run = end.saturating_sub(run_end);
            let worst = out_of_order.max(before_run).max(past_run);
            if worst > 0 {
                // FIRST violation only. `boyko_app` depends on `boyko_ecs`, so it calls the
                // reporter DIRECTLY rather than raising a sticky bit — the route the `92xx`
                // emitter's own L8c block prescribes for this crate, because `fold.rs` is the flag
                // word's single consumer and a bit raised by a run that outlives the last fold is a
                // report nobody takes. The reporter's `Once` latch makes it one record per process,
                // which is the stickiness a raised flag would have bought.
                if self.order.violations == 0 {
                    boyko_ecs::ecs::core::profiling::report_zone_order_violated(m, run_id);
                }
                self.order.violations += 1;
                let ns = (worst as f64) * self.period_ns;
                if ns > self.order.worst_ns {
                    self.order.worst_ns = ns;
                }
            }
            prev_begin = p.begin_ticks;
        }
    }

    /// Forms this frame's derived rows and pushes ONE sample each — VB-SV0 DP6-0b.
    ///
    /// The offsets pushed beside the value are the MINUEND's, so the row says where in the frame
    /// the quantity was formed. ⚠️ `end_off − begin_off` is therefore NOT the derived duration: the
    /// net is a bracket minus a hole inside it, which is not a contiguous interval and has no
    /// begin/end of its own. The row's `median_ns` is the quantity; the offsets locate it.
    fn fold_derived(&mut self, pairs: &[PairResult], base: Option<u64>) {
        for spec in self.derived {
            // The derived id is NEVER stamped — `TsWitness` has no site for it, `pair_of[14]` stays
            // `NO_PAIR` and mask bit 14 is never set. Asserted here, at the one place that would
            // otherwise compute a value and push it beside a bracket's, because the artifact cannot
            // express the difference: a derived row and a bracketed row are the same six numbers.
            debug_assert!(
                !pairs.iter().any(|p| p.zone == spec.zone),
                "invariant: zone {} is derived and has no recorder site, yet the frame's slice \
                 carries a pair for it",
                spec.zone
            );
            let (Term::Value(minuend), Some(minuend_pair)) = (
                resolve_term(pairs, spec.minuend, spec.minuend_expect),
                pairs.iter().find(|p| p.zone == spec.minuend),
            ) else {
                self.order.frames_skipped += 1;
                continue;
            };
            let subtrahend = match resolve_term(pairs, spec.subtrahend, spec.subtrahend_expect) {
                Term::Value(v) => v,
                Term::Zero => 0.0,
                Term::Skip => {
                    self.order.frames_skipped += 1;
                    continue;
                }
            };
            if subtrahend > minuend {
                // A negative net is not a measurement. The frame is skipped rather than clamped —
                // a clamp would publish a zero that reads as "this leg costs nothing" — and the
                // cause is already counted: a subtrahend longer than its container is a containment
                // violation, which `check_order` reported above.
                self.order.frames_skipped += 1;
                continue;
            }
            let value = (minuend - subtrahend) * self.period_ns;
            let (begin_off, end_off) = match base {
                Some(b) => {
                    let begin = (minuend_pair.begin_ticks.saturating_sub(b) as f64) * self.period_ns;
                    (begin, begin + minuend * self.period_ns)
                }
                None => (0.0, 0.0),
            };
            let idx = self.accum_index(spec.zone, ZoneLabel::Measured);
            let acc = &mut self.zones[idx];
            acc.dur_ns.push(value);
            acc.begin_off_ns.push(begin_off);
            acc.end_off_ns.push(end_off);
        }
    }

    /// This zone's accumulator, created at first sight with `label` as its opening worst.
    fn accum_index(&mut self, zone: u16, label: ZoneLabel) -> usize {
        match self.zones.iter().position(|z| z.zone == zone) {
            Some(i) => i,
            None => {
                self.zones.push(ZoneAccum {
                    zone,
                    worst: label,
                    dur_ns: Vec::new(),
                    begin_off_ns: Vec::new(),
                    end_off_ns: Vec::new(),
                });
                self.zones.len() - 1
            }
        }
    }

    /// Records one dropped pair against `boyko_diag`'s process-wide loss counter — **profiling
    /// rung 8, `G4c`: the loss has to reach the reader.**
    ///
    /// # Why HERE, and why only two of the four labels
    ///
    /// D13's rule is that counts originate AT the operation they count, and this is the one place
    /// in the tree that sees every retired pair's label. Recording it at the artifact writer instead
    /// would count what the writer was handed, not what the recorder observed.
    ///
    /// [`ZoneLabel::Lost`] and [`ZoneLabel::Torn`] are losses. **[`ZoneLabel::NotBracketed`] is
    /// NOT**, and the distinction is the whole reason the 2x2 label exists: a pair the recorder
    /// never opened is a STATED ABSENCE — this leg does not run that pass — while a lost pair is a
    /// bracket whose numbers went missing. Counting the first as loss would make "the VB family
    /// does not run on a Deferred frame" indistinguishable from "the query results never came
    /// back", and every artifact from every non-VB path would report drops it never had.
    ///
    /// The class is [`LossClass::Device`], whose own doc says why: *"the loss happened off-CPU (a
    /// GPU query pool, a driver ring) and the host learnt of it only afterwards, so the count is
    /// reconstructed rather than observed at the drop."* That is exactly what a retired pair is.
    ///
    /// `bytes = 0`: a lost timestamp pair has no payload figure that means anything. The field is
    /// documented as `0` for such classes rather than filled with the 16 bytes of query storage,
    /// which would be a number about the pool rather than about the loss.
    #[inline]
    fn record_device_loss() {
        boyko_diag::loss::record_here(boyko_diag::loss::LossClass::Device, 0);
    }

    /// Reduces the window.
    ///
    /// Rows come out sorted by zone id so the artifact is deterministic: two runs of the same
    /// configuration differ in their numbers, never in their row order, which is what lets a reader
    /// diff two files line for line.
    #[must_use]
    pub fn finish(mut self) -> (Vec<ZoneRow>, LabelCensus, OrderCensus) {
        self.zones.sort_unstable_by_key(|z| z.zone);
        // VB-SV0 DP6-0b: the derived rows' `n` FLOOR, resolved here because this is where both
        // numbers exist. A derived row folded over a materially different subset of frames than the
        // terms it is compared against is not comparable to them — so the file SAYS so, instead of
        // leaving every reader to re-derive `n < 0.9 * frames_checked` and one of them to forget.
        let floor = 0.9 * f64::from(self.order.frames_checked);
        let mut order = self.order;
        for spec in self.derived {
            let n = self.zones.iter().find(|z| z.zone == spec.zone).map_or(0, |z| z.dur_ns.len());
            if (n as f64) < floor {
                order.derived_inconclusive.push(spec.zone);
            }
        }
        let rows = self
            .zones
            .iter()
            .map(|z| {
                let (median_ns, mean_ns, p95_ns, stddev_ns) = stats_ns(&z.dur_ns);
                let (begin_off_ns, ..) = stats_ns(&z.begin_off_ns);
                let (end_off_ns, ..) = stats_ns(&z.end_off_ns);
                ZoneRow {
                    zone: z.zone,
                    label: z.worst,
                    // The count of MEASURED samples, not of frames: a row saying `n = 30` when
                    // three of the thirty were torn would be claiming thirty measurements.
                    n: z.dur_ns.len() as u32,
                    median_ns,
                    mean_ns,
                    p95_ns,
                    stddev_ns,
                    begin_off_ns,
                    end_off_ns,
                }
            })
            .collect();
        // `boyko-W9205`, landed at logging rung L8c. Profiling rung 8 reserved the code for this
        // and never emitted it, so a window that lost half its pairs produced rows whose `n` was
        // simply smaller — a reader comparing two windows had no way to tell "this leg is faster"
        // from "this leg's results did not come back".
        //
        // Reported at `finish` rather than at the increment in `observe_frame`, because the claim
        // is about the WINDOW: one report carrying the window's totals is the statement a reader
        // acts on, where a report per lost pair would be a storm describing itself.
        //
        // A DIRECT CALL and not `loss::raise`: `fold.rs` is the flag word's single consumer, and a
        // window that finishes at the end of a measured run is finishing after the last fold. A
        // raised bit there is a report nobody takes.
        if self.census.lost > 0 || self.census.torn > 0 {
            boyko_ecs::ecs::core::profiling::report_window_zones_lost(
                self.census.lost,
                self.census.torn,
                self.census.measured,
            );
        }
        (rows, self.census, order)
    }
}

/// `(median, mean, p95, stddev)`, ns — `runner.rs`'s `vb_bench_stats_ns`, convention for
/// convention, plus the population standard deviation rung 8 needs.
///
/// # Why the stddev is MEASURED and not derived from `p95 - median`
///
/// Rung 8's `se_floor` term is the propagated standard error of the medians a contrast is built
/// from, and an SE needs a spread. The three fields this reducer already published do not carry
/// one: recovering `sigma` from `(p95 - median) / 1.6449` assumes the samples are normal, which GPU
/// frame times are not — they are right-skewed with a hard floor at the hardware quantum. That
/// estimator would have made one of the band's four terms an assumption wearing a measurement's
/// name, which is this campaign's own most-repeated defect. Carrying the real second moment costs
/// one pass over a slice that is already in cache.
///
/// Returns zeros on an empty slice rather than asserting, because a zone the recorder never
/// measured is a normal outcome here: its row carries its label, and the label is what says the
/// numbers are not measurements. The shipped helper asserts instead because its caller could never
/// reach it empty.
#[must_use]
pub fn stats_ns(samples: &[f64]) -> (f64, f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let mut sorted: Vec<f64> = samples.to_vec();
    // `f64` is only `PartialOrd`; these are GPU timestamp deltas scaled by a finite period, so
    // `partial_cmp` cannot return `None` — say so rather than `unwrap()`.
    sorted.sort_unstable_by(|a, b| {
        a.partial_cmp(b).expect("invariant: GPU timestamp deltas are finite, never NaN")
    });
    let n = sorted.len();
    let median = if n % 2 == 1 { sorted[n / 2] } else { 0.5 * (sorted[n / 2 - 1] + sorted[n / 2]) };
    let p95 = sorted[((n as f64 * 0.95) as usize).min(n - 1)];
    // POPULATION sigma (divide by `n`), not the sample estimator (`n - 1`): the window is not a
    // sample drawn from a larger population of frames -- it IS every frame the sitting measured,
    // and the SE below is about the median of exactly these. A single-sample window gets `0.0`,
    // which is the true spread of one number rather than a division by zero.
    let var = sorted.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
    (median, mean, p95, var.sqrt())
}

#[cfg(test)]
mod tests {
    use boyko_rhi_vulkan::present::gpu_zone::ZONE_VB_SHADE;

    use super::*;

    /// The statistics agree with the shipped helper's conventions, including the even-count median
    /// and the p95 index — the property that keeps the artifact comparable with the printed lines.
    #[test]
    fn the_statistics_match_the_shipped_conventions() {
        // Even count: the median is the average of the two central values, not either of them.
        let (median, mean, p95, stddev) = stats_ns(&[10.0, 20.0, 30.0, 40.0]);
        assert!((median - 25.0).abs() < f64::EPSILON, "even-count median must average the centre");
        assert!((mean - 25.0).abs() < f64::EPSILON);
        // (4 * 0.95) as usize == 3 -> the last element.
        assert!((p95 - 40.0).abs() < f64::EPSILON);
        // POPULATION sigma of {10,20,30,40} about a mean of 25 is sqrt(500/4) = 11.18..., not the
        // sample estimator's sqrt(500/3) = 12.90. The distinction is asserted rather than left to
        // the implementation, because rung 8's `se_floor` divides by it.
        assert!((stddev - 125.0f64.sqrt()).abs() < 1e-12, "stddev must be the POPULATION sigma");

        // Odd count: the middle element itself.
        let (median, ..) = stats_ns(&[10.0, 20.0, 30.0]);
        assert!((median - 20.0).abs() < f64::EPSILON);
    }

    /// An empty zone reduces to zeros rather than panicking, and its ROW still carries a label.
    #[test]
    fn a_zone_with_no_measured_sample_reduces_to_zeros() {
        assert_eq!(stats_ns(&[]), (0.0, 0.0, 0.0, 0.0));
    }

    /// Offsets are relative to each frame's OWN base, so a drifting GPU clock does not leak in.
    #[test]
    fn offsets_are_taken_from_each_frames_own_base() {
        let mut r = WindowReducer::new(1.0, &[], &[]);
        // Two frames, same shape, bases a million ticks apart.
        for base in [1_000u64, 1_000_000u64] {
            r.observe_frame(&[
                PairResult { zone: 16, label: GpuLabel::Measured, begin_ticks: base, dur_ticks: 100 },
                PairResult {
                    zone: 17,
                    label: GpuLabel::Measured,
                    begin_ticks: base + 500,
                    dur_ticks: 50,
                },
            ]);
        }
        let (rows, census, _order) = r.finish();
        assert_eq!(rows.len(), 2);
        assert_eq!(census.measured, 4);
        assert_eq!(rows[0].zone, 16);
        assert!(rows[0].begin_off_ns.abs() < f64::EPSILON, "the base zone's offset must be 0");
        assert!(
            (rows[1].begin_off_ns - 500.0).abs() < f64::EPSILON,
            "the second zone's offset must be 500 in BOTH frames, so its median is 500 — a \
             reducer that folded raw begin_ticks would report roughly half a million here"
        );
        assert!((rows[1].end_off_ns - 550.0).abs() < f64::EPSILON);
    }

    /// The worst label in the window survives to the row.
    #[test]
    fn one_torn_frame_makes_the_whole_window_torn_for_that_zone() {
        let mut r = WindowReducer::new(1.0, &[], &[]);
        r.observe_frame(&[PairResult {
            zone: 16,
            label: GpuLabel::Measured,
            begin_ticks: 10,
            dur_ticks: 100,
        }]);
        r.observe_frame(&[PairResult {
            zone: 16,
            label: GpuLabel::Torn,
            begin_ticks: 0,
            dur_ticks: 0,
        }]);
        let (rows, census, _order) = r.finish();
        assert_eq!(rows[0].label, ZoneLabel::Torn, "a window with a torn frame is not `Measured`");
        assert_eq!(rows[0].n, 1, "n counts MEASURED samples, not frames");
        assert_eq!(census.measured, 1);
        assert_eq!(census.torn, 1);
    }

    /// Rows come out sorted by zone id, so two runs differ in numbers and never in row order.
    #[test]
    fn rows_are_ordered_by_zone_id() {
        let mut r = WindowReducer::new(1.0, &[], &[]);
        r.observe_frame(&[
            PairResult { zone: 32, label: GpuLabel::Measured, begin_ticks: 0, dur_ticks: 1 },
            PairResult { zone: 16, label: GpuLabel::Measured, begin_ticks: 0, dur_ticks: 1 },
        ]);
        let (rows, _, _) = r.finish();
        assert!(rows[0].zone < rows[1].zone, "rows must be sorted, they came out {rows:?}");
    }

    // ===========================================================================================
    // VB-SV0 DP6-0b — the per-frame chain and the derived row
    // ===========================================================================================

    /// A `Measured` pair, spelled once so the frames below read as the shapes they are.
    fn m(zone: u16, begin: u64, dur: u64) -> PairResult {
        PairResult { zone, label: GpuLabel::Measured, begin_ticks: begin, dur_ticks: dur }
    }

    /// One healthy split frame: `12b` … `10` … `11` … `13` … `2` … `12e`, all nested in the run.
    fn healthy_split_frame() -> Vec<PairResult> {
        vec![
            m(ZONE_VB_PRODUCE_RUN, 1_000, 10_000),
            m(ZONE_VB_SDF_MESH, 1_100, 300),
            m(ZONE_VB_GEO, 2_000, 500),
            m(ZONE_VB_PRESHADE, 3_000, 6_000),
            m(ZONE_VB_SHADE, 9_000, 1_500),
        ]
    }

    /// The chain holds on a well-ordered frame, and the census SAYS how many frames it checked —
    /// the number without which `violations == 0` is not readable.
    #[test]
    fn a_well_ordered_frame_violates_nothing_and_is_counted() {
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, &[]);
        r.observe_frame(&healthy_split_frame());
        r.observe_frame(&healthy_split_frame());
        let (_, _, order) = r.finish();
        assert_eq!(order.violations, 0);
        assert_eq!(order.frames_checked, 2, "the run bracket measured on both frames");
    }

    /// **The red the whole channel exists for**: one member's BEGIN moves ahead of its
    /// predecessor's — what a `TOP_OF_PIPE` stamp does non-deterministically — and the COUNT sees
    /// it even though a median over the window would not.
    #[test]
    fn one_out_of_order_frame_in_a_window_is_counted() {
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, &[]);
        for _ in 0..9 {
            r.observe_frame(&healthy_split_frame());
        }
        let mut bad = healthy_split_frame();
        // id 11 latches 500 ticks BEFORE id 10, the overlap a TOP begin produces.
        bad[2].begin_ticks = 600;
        r.observe_frame(&bad);
        let (_, _, order) = r.finish();
        assert_eq!(order.frames_checked, 10);
        assert_eq!(order.violations, 1, "one frame in ten, and the count must not average it away");
        assert!(order.worst_ns > 0.0, "the worst delta must be published beside the count");
    }

    /// A member that runs PAST its containing run is a violation too, on the same count.
    #[test]
    fn a_member_escaping_the_run_bracket_is_counted() {
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, &[]);
        let mut bad = healthy_split_frame();
        // Id 13 (a CHAIN MEMBER — index 3) ends at 3 000 + 9 000 = 12 000, past the run's 11 000.
        // Deliberately not id 2: that one is `TOP`-stamped, is not in the chain, and mutating it
        // would make this test pass or fail on a zone the census does not read.
        assert_eq!(bad[3].zone, ZONE_VB_PRESHADE, "this test mutates the pre-shade bracket");
        bad[3].dur_ticks = 9_000;
        r.observe_frame(&bad);
        let (_, _, order) = r.finish();
        assert_eq!(order.violations, 1);
        assert!((order.worst_ns - 1_000.0).abs() < f64::EPSILON, "worst was {}", order.worst_ns);
    }

    /// **Id 2 is not a chain member, and that is asserted rather than left to the static's shape.**
    ///
    /// A `TOP_OF_PIPE` begin retires at command FETCH, so `b2` can legitimately land before a
    /// `BOTTOM` member's begin on the same frame. Ordering it against them would emit violations
    /// that describe the stage difference — non-deterministically. This drives exactly that frame
    /// and requires ZERO violations.
    #[test]
    fn the_top_stamped_shade_bracket_is_not_ordered_against_the_bottom_members() {
        assert!(
            !VB_CHAIN_SPLIT.contains(&ZONE_VB_SHADE) && !VB_CHAIN_FUSED.contains(&ZONE_VB_SHADE),
            "id 2 keeps TOP_OF_PIPE, so it must not be a member of a BOTTOM-stamped chain"
        );
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, &[]);
        let mut frame = healthy_split_frame();
        // `b2` retires at fetch: ahead of id 10, id 11 and id 13, and before the run's own begin.
        frame[4].begin_ticks = 500;
        r.observe_frame(&frame);
        let (_, _, order) = r.finish();
        assert_eq!(order.frames_checked, 1);
        assert_eq!(
            order.violations, 0,
            "a TOP-stamped begin ahead of every BOTTOM member is the stage difference, not a \
             record-order defect, and the census must not report it as one"
        );
    }

    /// Ties are legal: two `BOTTOM_OF_PIPE` stamps on the same tick are an empty bracket, not a
    /// breach. This box's empty-bracket floor is 96–128 ticks and a strict `<` would red on it.
    #[test]
    fn equal_begins_are_not_a_violation() {
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, &[]);
        let mut frame = healthy_split_frame();
        frame[2].begin_ticks = frame[1].begin_ticks;
        frame[2].dur_ticks = 0;
        r.observe_frame(&frame);
        let (_, _, order) = r.finish();
        assert_eq!(order.violations, 0);
    }

    /// **A window with no declared chain publishes `frames_checked == 0`**, which is what makes a
    /// silent pass impossible: the gate reads the two numbers together.
    #[test]
    fn an_undeclared_chain_checks_nothing_and_says_so() {
        let mut r = WindowReducer::new(1.0, &[], &[]);
        r.observe_frame(&healthy_split_frame());
        let (_, _, order) = r.finish();
        assert_eq!(order.frames_checked, 0);
        assert_eq!(order.violations, 0);
    }

    /// The derived row is `minuend − subtrahend`, formed at the frame and reduced afterwards.
    #[test]
    fn the_derived_row_is_the_difference_of_the_two_brackets() {
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, VB_DERIVED_SPLIT);
        r.observe_frame(&healthy_split_frame());
        r.observe_frame(&healthy_split_frame());
        let (rows, _, order) = r.finish();
        let net = rows.iter().find(|z| z.zone == ZONE_VB_PRODUCE_NET).expect("the derived row");
        assert!((net.median_ns - 4_000.0).abs() < f64::EPSILON, "10 000 - 6 000, got {net:?}");
        assert_eq!(net.n, 2, "one sample per frame, never a composition of two medians");
        assert!(order.derived_inconclusive.is_empty(), "n == frames_checked clears the floor");
    }

    /// **The absence policy, both readings of the SAME absence.**
    ///
    /// A fused leg declares `PRESHADE` `Forbidden`, so its absence is structural and contributes
    /// zero — `NET ≡ PRODUCE_RUN`. A split leg declares it `Required`, so the same absence means
    /// the bracket was refused while its work ran, and the frame is SKIPPED rather than credited
    /// with a 10 000-tick net it never had.
    #[test]
    fn the_same_absence_contributes_zero_or_skips_the_frame_by_leg() {
        let fused_frame = vec![
            m(ZONE_VB_PRODUCE_RUN, 1_000, 10_000),
            m(ZONE_VB_SDF_MESH, 1_100, 300),
            m(ZONE_VB_SHADE, 5_000, 1_500),
        ];

        let mut fused = WindowReducer::new(1.0, VB_CHAIN_FUSED, VB_DERIVED_FUSED);
        fused.observe_frame(&fused_frame);
        let (rows, _, order) = fused.finish();
        let net = rows.iter().find(|z| z.zone == ZONE_VB_PRODUCE_NET).expect("the derived row");
        assert!((net.median_ns - 10_000.0).abs() < f64::EPSILON, "NET must equal PRODUCE_RUN");
        assert_eq!(order.frames_skipped, 0);

        // The SAME frame, read against the split leg's expectations.
        let mut split = WindowReducer::new(1.0, VB_CHAIN_SPLIT, VB_DERIVED_SPLIT);
        split.observe_frame(&fused_frame);
        let (rows, _, order) = split.finish();
        assert!(
            rows.iter().all(|z| z.zone != ZONE_VB_PRODUCE_NET),
            "a Required subtrahend that is absent must skip the frame, not push a sample"
        );
        assert_eq!(order.frames_skipped, 1);
        assert!(
            order.derived_inconclusive.contains(&ZONE_VB_PRODUCE_NET),
            "a derived row with no samples over a checked frame is INCONCLUSIVE, not absent"
        );
    }

    /// **A stamped derived id is refused rather than merged.**
    ///
    /// The failure guarded against is silent: a `PairResult` under id 14 would land in the SAME
    /// accumulator the derived difference is pushed into, and the published row would reduce a
    /// mixture of a bracket duration and a difference under one key, with no label able to say so.
    ///
    /// `#[should_panic]` because the dev profile asserts first — `fold_derived` runs before the
    /// accumulation loop. In RELEASE the assertion is gone and the loop's skip is what ships; that
    /// arm cannot be driven from a debug test harness, which is stated here rather than implied by
    /// a test that seems to cover it.
    ///
    /// # ⚠️ IGNORED in release, and the profile is not incidental
    ///
    /// `debug_assert!` compiles to nothing when `debug_assertions` is off, so under `--release`
    /// this test panics nowhere and `#[should_panic]` FAILS it. **Release is the profile every DP6
    /// gate runs in** — the same fact that made `TsWitness::writes` unusable as the unmatched-END
    /// detector — so left ungated this test would red the gate it belongs to, on a build where the
    /// code under test is behaving exactly as designed.
    ///
    /// Gated rather than rewritten to assert the skip instead: the release arm's own doc says it is
    /// undrivable from a test harness (the assertion in `fold_derived` fires first in every profile
    /// that has assertions at all), so a "release version" of this test would have to assert
    /// something other than what it is named for.
    #[test]
    #[should_panic(expected = "is derived and has no recorder site")]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "the guard under test is a `debug_assert!`, which is compiled out in release; \
                  the release path is the accumulation loop's skip arm, and that arm is \
                  undrivable from a test harness because `fold_derived` asserts first in every \
                  profile that keeps assertions"
    )]
    fn a_stamped_derived_id_is_refused() {
        let mut frame = healthy_split_frame();
        frame.push(m(ZONE_VB_PRODUCE_NET, 2_000, 400));
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, VB_DERIVED_SPLIT);
        r.observe_frame(&frame);
    }

    /// A term that is PRESENT but not `Measured` skips the frame whatever the expectation says: the
    /// recorder opened that bracket, so the work ran and only its number is missing.
    #[test]
    fn a_present_but_unmeasured_term_skips_the_frame() {
        let mut frame = healthy_split_frame();
        frame[3].label = GpuLabel::Lost;
        let mut r = WindowReducer::new(1.0, VB_CHAIN_SPLIT, VB_DERIVED_SPLIT);
        r.observe_frame(&frame);
        let (rows, _, order) = r.finish();
        assert!(rows.iter().all(|z| z.zone != ZONE_VB_PRODUCE_NET));
        assert_eq!(order.frames_skipped, 1);
    }
}
