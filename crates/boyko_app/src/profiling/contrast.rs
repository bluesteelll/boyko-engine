//! Profiling rung 8 — **the comparator.** `Floor`, `Twin`, `resolve`, and the verdict that is the
//! only thing this subsystem is allowed to say about two measurements.
//!
//! # The one question this module exists to make unanswerable
//!
//! *"Just give me the delta."* [`resolve`] returns [`Contrast::Resolved`] or
//! [`Contrast::NotResolved`]; there is no third variant, and **there is no constructor anywhere in
//! this module that produces a bare delta**. A caller who wants a number without a verdict has to
//! read the fields off a `NotResolved`, which carries them — deliberately, so the refusal is
//! informative — and in doing so has to look at the reason.
//!
//! # Why this lives in `boyko_app` and not in `boyko_ecs`
//!
//! The corpus places `Floor` in `boyko_ecs::…::profiling::floor`, in the same breath as
//! `WindowReducer` and the TOML artifact. Rung 7 moved those two into this crate, and its reasoning
//! transfers unchanged: a [`Floor`] is read from a file the artifact writer produced, and a
//! [`LegSummary`] is built from an [`Artifact`], whose header carries a `workload_tag` derived from
//! `boyko_render::ResolvedRenderPath`. The kernel cannot see that type and must not learn to. The
//! departure is recorded here rather than argued in a commit message, because a reader looking for
//! `boyko_ecs::profiling::floor` needs to find out where it went from the place it is not.
//!
//! # The band is four terms and no single one of them is the band
//!
//! ```text
//! band = max( floor.rel * |median_a| ,   // cross-process 3σ CV of THIS workload, THIS box
//!             twin.ticks             ,   // in-sitting drift, from the interleaved zero control
//!             se_floor(a, b)         ,   // propagated SE of the medians the delta is built from
//!             quantum                )   // the instrument's own resolution — a sub-floor, never
//!                                        // the whole band
//! ```
//!
//! Each term answers a different failure. Dropping any of them leaves a way to report a win: a
//! floor alone misses drift within the sitting, a twin alone misses that the box is different
//! today, an SE alone misses both, and a quantum alone is a tolerance of zero on hardware whose
//! counter steps in 96–128-tick jumps.
//!
//! # What this module CANNOT claim
//!
//! It cannot claim the floor file was measured honestly — only that the API cannot manufacture one.
//! [`Floor::from_session_file`] is the sole constructor, [`FLOOR_SIGMA`] is a `const`, and
//! [`FLOOR_REDUCTION`] is a `const` the constructor applies with no parameter. That is a constraint
//! on the shape of a lie, not proof of a truth.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::artifact::{Artifact, ArtifactError, ZoneLabel};

/// Three sigma. **There is no caller-supplied sigma anywhere in this API**, which is the whole of
/// what rev 2's `Floor::from_aa_control(control, sigma)` got wrong: a caller who may choose the
/// sigma may choose the verdict.
pub const FLOOR_SIGMA: f64 = 3.0;

/// Separate processes per repetition, per condition — `vg_decidability_floor.rs`'s protocol.
pub const FLOOR_SESSIONS: u32 = 7;

/// Independent repetitions of that whole protocol. All three are published; none is averaged away.
pub const FLOOR_REPEATS: u32 = 3;

/// How the three repetition floors collapse to the one scalar [`resolve`] reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reduction {
    /// The worst repetition.
    Max,
    /// The luckiest. **Exists only so a gate can name it**; production cannot select it.
    Min,
}

impl Reduction {
    /// Applies the reduction, returning `(value, index)` — the index is what the artifact prints as
    /// `rel_source_repeat`, so a reader can go back to the repetition that supplied the number.
    #[must_use]
    fn apply(self, all: &[f64]) -> (f64, u32) {
        let mut best = (all[0], 0u32);
        for (i, &v) in all.iter().enumerate().skip(1) {
            let take = match self {
                Reduction::Max => v > best.0,
                Reduction::Min => v < best.0,
            };
            if take {
                best = (v, i as u32);
            }
        }
        best
    }
}

/// **The honest scalar.** A `const`, applied by [`Floor::from_session_file`] with no parameter.
///
/// A floor is a claim about what this instrument *cannot* decide. The honest scalar for that claim
/// is the WORST repetition, not the luckiest and not their average. The choice is load-bearing and
/// was measured: this protocol's own repetitions span **6.3 / 14.3 / 4.7 / 13.5 %** on this box, a
/// 3× difference between the candidate reductions — so `Min` or a mean rebuilds the false-win
/// machine at a different scale **while satisfying every arithmetic check**. `G3a`'s reduction RED
/// changes this value and nothing else.
///
/// "Never averaged" is preserved and is a DIFFERENT statement from "never reduced": [`Floor`]
/// carries all three in [`Floor::rel_all`] and names the one it used in
/// [`Floor::rel_source_repeat`]. What is forbidden is collapsing them by averaging, which invents a
/// value no repetition measured.
pub const FLOOR_REDUCTION: Reduction = Reduction::Max;

/// Identity of the thing measured — hashed so two legs' tags compare in one word.
///
/// Covers all three halves the corpus names: the DERIVED config identity (`workload_tag`, FNV-1a
/// over the whole boot-resolved render path), the DECLARED content (`content_tag`, which no engine
/// can derive — light count, scene, rig), and the SUBSCRIBED ZONE SET, because a contrast over
/// zones 0–2 and one over zones 0–9 are not the same measurement even on one configuration.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct WorkloadTag(u64);

impl WorkloadTag {
    /// FNV-1a, the mint this repository already uses for `config_tag`.
    #[must_use]
    fn hash_bytes(mut h: u64, bytes: &[u8]) -> u64 {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// Builds the tag from an artifact and the zone ids a reading subscribed to.
    ///
    /// `zones` is sorted internally, so two callers that named the same set in different orders get
    /// the same tag — the set is what matters, and a caller's iteration order is not a property of
    /// the measurement.
    #[must_use]
    pub fn of(artifact: &Artifact, zones: &[u16]) -> WorkloadTag {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        h = Self::hash_bytes(h, artifact.header.workload_tag.as_bytes());
        h = Self::hash_bytes(h, b"\x1f");
        h = Self::hash_bytes(h, artifact.header.content_tag.as_bytes());
        h = Self::hash_bytes(h, b"\x1f");
        let mut sorted: Vec<u16> = zones.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        for z in sorted {
            h = Self::hash_bytes(h, &z.to_le_bytes());
        }
        WorkloadTag(h)
    }

    /// The raw word, for printing. Not a constructor: there is no `WorkloadTag::from_u64`, so a tag
    /// can only come from an artifact that was actually written.
    #[must_use]
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// **The cross-process floor** — the smallest defensible RELATIVE delta for this workload, this
/// box, this protocol.
///
/// Not a quantum (that is the instrument's resolution) and not a twin (that is in-sitting drift).
/// Conflating the three is what rev 1 and rev 2 each did in a different place.
#[derive(Clone, PartialEq, Debug)]
pub struct Floor {
    /// [`FLOOR_REDUCTION`] over [`Self::rel_all`]. A fraction, not a percentage.
    rel: f64,
    /// Every repetition floor, in measurement order. Published, never averaged.
    rel_all: Vec<f64>,
    /// Which entry of [`Self::rel_all`] supplied [`Self::rel`].
    rel_source_repeat: u32,
    /// What was measured. `resolve` refuses a floor whose tag differs from the leg's.
    workload: WorkloadTag,
    /// Sessions per repetition, as the file records them.
    sessions: u32,
    /// Repetitions, as the file records them.
    repeats: u32,
    /// Where it came from, so a verdict can name its own evidence.
    path: PathBuf,
}

/// Why a session file was refused.
#[derive(Debug)]
pub enum FloorError {
    /// The file could not be read.
    Io(io::Error),
    /// A line was malformed, or a required key was missing.
    Malformed {
        /// 1-based line, or `0` when the fault is a missing key rather than a bad line.
        line: usize,
        /// What was wrong.
        why: &'static str,
    },
    /// The file records a protocol this build does not accept.
    ProtocolMismatch {
        /// What the file said.
        found: (u32, u32),
        /// What [`FLOOR_SESSIONS`]/[`FLOOR_REPEATS`] require.
        expected: (u32, u32),
    },
}

impl From<io::Error> for FloorError {
    fn from(e: io::Error) -> FloorError {
        FloorError::Io(e)
    }
}

impl core::fmt::Display for FloorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FloorError::Io(e) => write!(f, "floor session file: {e}"),
            FloorError::Malformed { line, why } => {
                write!(f, "floor session file: malformed at line {line}: {why}")
            }
            FloorError::ProtocolMismatch { found, expected } => write!(
                f,
                "floor session file records {} sessions x {} repeats; this build's protocol is \
                 {} x {}. A floor measured under a different protocol bounds nothing about this one",
                found.0, found.1, expected.0, expected.1
            ),
        }
    }
}

impl std::error::Error for FloorError {}

impl Floor {
    /// **THE ONLY CONSTRUCTOR.**
    ///
    /// Deleted in rev 3 and never reinstated: `Floor::from_aa_control(control, sigma)`, which took
    /// a single in-sitting control and a caller-chosen sigma. Never existed: `Floor::from_quantum`.
    /// Both are named here so a future reader can see that their absence is a decision.
    ///
    /// The file is written by the measuring protocol itself (`vg_decidability_floor_measure`), so
    /// the only way to obtain a `Floor` is to have run 21 processes per condition.
    ///
    /// # Errors
    ///
    /// [`FloorError::Io`] if unreadable, [`FloorError::Malformed`] on a bad or missing key, and
    /// [`FloorError::ProtocolMismatch`] when the recorded protocol is not this build's.
    pub fn from_session_file(path: &Path) -> Result<Floor, FloorError> {
        let text = fs::read_to_string(path)?;
        let mut rel_all: Vec<f64> = Vec::new();
        let mut workload: Option<u64> = None;
        let mut sessions: Option<u32> = None;
        let mut repeats: Option<u32> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(FloorError::Malformed { line: i + 1, why: "not a `key = value` line" });
            };
            let (k, v) = (k.trim(), v.trim().trim_matches('"'));
            let bad = |why| FloorError::Malformed { line: i + 1, why };
            match k {
                "rel" => rel_all.push(v.parse().map_err(|_| bad("`rel` is not a float"))?),
                "workload" => {
                    workload = Some(v.parse().map_err(|_| bad("`workload` is not a u64"))?);
                }
                "sessions" => {
                    sessions = Some(v.parse().map_err(|_| bad("`sessions` is not a u32"))?);
                }
                "repeats" => repeats = Some(v.parse().map_err(|_| bad("`repeats` is not a u32"))?),
                // An unknown key is IGNORED rather than refused: the writer may publish provenance
                // this reader does not consume, and refusing it would couple the two files' shapes
                // for no gain. A MISSING required key still reds, below.
                _ => {}
            }
        }
        let miss = |why| FloorError::Malformed { line: 0, why };
        let workload = WorkloadTag(workload.ok_or_else(|| miss("no `workload`"))?);
        let sessions = sessions.ok_or_else(|| miss("no `sessions`"))?;
        let repeats = repeats.ok_or_else(|| miss("no `repeats`"))?;
        if rel_all.is_empty() {
            return Err(miss("no `rel` line: a floor with no repetition measured nothing"));
        }
        if rel_all.len() as u32 != repeats {
            return Err(miss("`repeats` disagrees with the number of `rel` lines"));
        }
        if (sessions, repeats) != (FLOOR_SESSIONS, FLOOR_REPEATS) {
            return Err(FloorError::ProtocolMismatch {
                found: (sessions, repeats),
                expected: (FLOOR_SESSIONS, FLOOR_REPEATS),
            });
        }
        // The reduction is applied HERE, by a `const`, with no parameter. A caller cannot reach it.
        let (rel, rel_source_repeat) = FLOOR_REDUCTION.apply(&rel_all);
        Ok(Floor {
            rel,
            rel_all,
            rel_source_repeat,
            workload,
            sessions,
            repeats,
            path: path.to_path_buf(),
        })
    }

    /// The reduced scalar the band uses.
    #[must_use]
    pub fn rel(&self) -> f64 {
        self.rel
    }

    /// Every repetition floor. Published so a report can print all three.
    #[must_use]
    pub fn rel_all(&self) -> &[f64] {
        &self.rel_all
    }

    /// Which repetition supplied [`Self::rel`].
    #[must_use]
    pub fn rel_source_repeat(&self) -> u32 {
        self.rel_source_repeat
    }

    /// What this floor was measured on.
    #[must_use]
    pub fn workload(&self) -> WorkloadTag {
        self.workload
    }

    /// Sessions per repetition, as recorded.
    #[must_use]
    pub fn sessions(&self) -> u32 {
        self.sessions
    }

    /// Repetitions, as recorded.
    #[must_use]
    pub fn repeats(&self) -> u32 {
        self.repeats
    }

    /// The file this floor came from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Renders a session file — the writer that makes [`Self::from_session_file`] reachable.
    ///
    /// Lives beside the reader so the two cannot drift, and takes the repetition floors rather than
    /// a `Floor`, because a `Floor` is what this function's OUTPUT is used to build. There is
    /// deliberately no `Floor -> file` round trip: that would let a caller reduce, edit, and
    /// re-publish a scalar no repetition measured.
    #[must_use]
    pub fn render_session_file(rel_all: &[f64], workload: WorkloadTag, sessions: u32) -> String {
        use core::fmt::Write as _;
        let mut s = String::with_capacity(256);
        let _ = writeln!(s, "# Machine-written by the decidability-floor protocol. Do not hand-edit:");
        let _ = writeln!(s, "# every `rel` here is a measurement, and the reduction is a const.");
        let _ = writeln!(s, "workload = {}", workload.as_u64());
        let _ = writeln!(s, "sessions = {sessions}");
        let _ = writeln!(s, "repeats = {}", rel_all.len());
        for r in rel_all {
            let _ = writeln!(s, "rel = {r}");
        }
        s
    }
}

/// **The in-sitting zero control** — ongoing drift while the contrast was being taken.
///
/// A different quantity from a [`Floor`]: the floor says what this box could not decide across
/// separate processes on some earlier day, the twin says what it was doing during THIS sitting.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Twin {
    /// The drift, in ns — `max(|median|, p90(|·|))` of the interleaved zero-control leg.
    ns: f64,
    /// How many rounds the control ran. Carried so a report can say how much evidence it rests on.
    rounds: u32,
    /// What it was measured beside.
    workload: WorkloadTag,
}

impl Twin {
    /// **The only constructor, and it takes no sigma.**
    ///
    /// The reduction is `max(|median|, p90|·|)` and it is fixed here for [`FLOOR_REDUCTION`]'s
    /// reason: a caller who may choose how a control is reduced may choose the band.
    #[must_use]
    pub fn from_zero_control(zero_control: &LegSummary) -> Twin {
        let m = zero_control.median_ns.abs();
        let p90 = zero_control.p95_ns.abs();
        Twin {
            ns: if m > p90 { m } else { p90 },
            rounds: zero_control.n,
            workload: zero_control.workload,
        }
    }

    /// The drift term, ns.
    #[must_use]
    pub fn ns(&self) -> f64 {
        self.ns
    }

    /// Rounds behind it.
    #[must_use]
    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// What it was measured beside.
    #[must_use]
    pub fn workload(&self) -> WorkloadTag {
        self.workload
    }
}

/// One leg of a contrast, reduced from an artifact.
///
/// Built by [`Self::from_artifact`] rather than by a literal, so a leg always carries the label and
/// census state that decide whether its numbers are numbers at all.
#[derive(Clone, PartialEq, Debug)]
pub struct LegSummary {
    /// What this leg measured.
    pub workload: WorkloadTag,
    /// The subscribed zones' summed median, ns — the leg's reading.
    pub median_ns: f64,
    /// Their summed p95, ns.
    pub p95_ns: f64,
    /// Propagated population sigma over the subscribed zones, ns.
    pub stddev_ns: f64,
    /// Samples behind the smallest-`n` subscribed zone — the honest count for the sum, since a sum
    /// is only as well-measured as its worst term.
    pub n: u32,
    /// The instrument's own resolution for this leg, ns.
    pub quantum_ns: f64,
    /// The process this leg came from, so a contrast can refuse two legs from different clocks.
    pub clock_epoch: (u64, u64),
    /// `true` when every subscribed zone came back `Measured`.
    pub all_measured: bool,
    /// `true` when the window recorded no drop of any class.
    pub window_complete: bool,
}

impl LegSummary {
    /// Reduces `artifact`'s subscribed `zones` into one leg.
    ///
    /// # The zones are SUMMED, and the alternative was rejected
    ///
    /// A contrast asks "is A faster than B", and the answer is about the whole subscribed extent,
    /// not about one zone at a time. Summing medians is not the median of the sum — the campaign
    /// already measured that `median(off) + median(dur) != median(off + dur)` — so this is stated
    /// as what it is: **a sum of per-zone medians, which is the right statistic for a total only if
    /// the zones are the partition their ids say they are.** For the VB family that holds by
    /// construction: `ZONE_VB_RUN`'s span is exactly the eight `BOTTOM_OF_PIPE` stamps between its
    /// own ends. A caller subscribing to an overlapping set gets a number that double-counts, which
    /// is why the subscribed set is part of the [`WorkloadTag`].
    ///
    /// # Errors
    ///
    /// [`ArtifactError::UndeclaredContent`] when the artifact does not say what it measured — the
    /// same refusal [`Artifact::floor_source`] makes, applied to every leg and not only to floors.
    pub fn from_artifact(
        artifact: &Artifact,
        zones: &[u16],
        quantum_ns: f64,
    ) -> Result<LegSummary, ArtifactError> {
        artifact.floor_source()?;
        let workload = WorkloadTag::of(artifact, zones);
        let mut median_ns = 0.0;
        let mut p95_ns = 0.0;
        let mut var_ns = 0.0;
        let mut n = u32::MAX;
        let mut all_measured = true;
        for &want in zones {
            match artifact.zones.iter().find(|z| z.zone == want) {
                Some(row) => {
                    median_ns += row.median_ns;
                    p95_ns += row.p95_ns;
                    // Variances add; sigmas do not. Independence between zones is an ASSUMPTION and
                    // it is the one place this reduction makes one — stated rather than hidden,
                    // because consecutive GPU passes on one queue are not independent and this term
                    // is therefore a LOWER bound on the true spread of the sum.
                    var_ns += row.stddev_ns * row.stddev_ns;
                    n = n.min(row.n);
                    all_measured &= row.label == ZoneLabel::Measured;
                }
                // A subscribed zone with no row is not a zero: it is a zone this window never saw,
                // which makes every number below a sum over a different set than the caller named.
                None => all_measured = false,
            }
        }
        let c = &artifact.census;
        Ok(LegSummary {
            workload,
            median_ns,
            p95_ns,
            stddev_ns: var_ns.sqrt(),
            n: if n == u32::MAX { 0 } else { n },
            quantum_ns,
            clock_epoch: (artifact.header.session_lo, artifact.header.session_hi),
            all_measured,
            window_complete: c.lost == 0 && c.torn == 0,
        })
    }
}

/// Why [`resolve`] refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotResolvedReason {
    /// `|median_delta| <= band`. **The common case, and not an error.**
    BelowBand,
    /// The floor was measured on something else.
    FloorWorkloadMismatch,
    /// So was the twin.
    TwinWorkloadMismatch,
    /// A leg's window dropped samples, so its median is over an unknown subset.
    WindowIncomplete,
    /// The legs came from different processes, so their clocks share no origin.
    ///
    /// Named `EpochBreak` and not `ClockEpochBreak`. The corpus uses both — its D11 table says one,
    /// its own `enum` and its S8 row say the other — and **the choice is recorded as arbitrary**:
    /// two of three source sites spell it this way. It is exactly the kind of one-word divergence a
    /// reviewer of a 1957-line document does not see, which is why it is settled in code.
    EpochBreak,
    /// Some subscribed zone came back `Lost`, `Torn` or `NotBracketed`.
    LabelNotMeasured,
}

impl NotResolvedReason {
    /// The wire word, for artifacts and reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            NotResolvedReason::BelowBand => "below_band",
            NotResolvedReason::FloorWorkloadMismatch => "floor_workload_mismatch",
            NotResolvedReason::TwinWorkloadMismatch => "twin_workload_mismatch",
            NotResolvedReason::WindowIncomplete => "window_incomplete",
            NotResolvedReason::EpochBreak => "epoch_break",
            NotResolvedReason::LabelNotMeasured => "label_not_measured",
        }
    }

    /// The inverse, for the round-trip `G4c` reads.
    ///
    /// Named `from_wire` and not `from_str`: clippy refuses an inherent `from_str` because it reads
    /// as `std::str::FromStr::from_str` at a call site while obeying none of that trait's contract
    /// — no `Err` type, no `parse()` integration. Implementing the real trait would buy `"x".parse()`
    /// and an error type nobody here needs; the wire word is an artifact field, not user input.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<NotResolvedReason> {
        Some(match s {
            "below_band" => NotResolvedReason::BelowBand,
            "floor_workload_mismatch" => NotResolvedReason::FloorWorkloadMismatch,
            "twin_workload_mismatch" => NotResolvedReason::TwinWorkloadMismatch,
            "window_incomplete" => NotResolvedReason::WindowIncomplete,
            "epoch_break" => NotResolvedReason::EpochBreak,
            "label_not_measured" => NotResolvedReason::LabelNotMeasured,
            _ => return None,
        })
    }
}

/// Which term of the band was the binding one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BandTerm {
    /// The cross-process floor.
    Floor,
    /// In-sitting drift.
    Twin,
    /// Propagated standard error.
    StandardError,
    /// The instrument's resolution.
    Quantum,
}

impl BandTerm {
    /// The wire word.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BandTerm::Floor => "floor",
            BandTerm::Twin => "twin",
            BandTerm::StandardError => "standard_error",
            BandTerm::Quantum => "quantum",
        }
    }
}

/// **The verdict.** There is no third variant and no bare-delta constructor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Contrast {
    /// The delta cleared the band.
    Resolved {
        /// `median_b - median_a`, ns. Negative means B is faster.
        median_delta_ns: f64,
        /// The band it cleared.
        band_ns: f64,
        /// Which term set the band.
        binding: BandTerm,
    },
    /// The delta did not clear the band, or the inputs did not license a comparison.
    ///
    /// **The delta fields are still populated.** A refusal that hid its numbers would be read as an
    /// error rather than as a measurement, and the operator would go looking for the delta
    /// elsewhere — which is the behaviour a comparator exists to prevent.
    NotResolved {
        /// Why.
        reason: NotResolvedReason,
        /// The delta anyway, ns.
        median_delta_ns: f64,
        /// The band anyway, ns. Zero when the refusal happened before a band could be computed.
        band_ns: f64,
        /// Which term set that band.
        binding: BandTerm,
    },
}

impl Contrast {
    /// The delta, whichever variant this is.
    #[must_use]
    pub fn median_delta_ns(&self) -> f64 {
        match *self {
            Contrast::Resolved { median_delta_ns, .. }
            | Contrast::NotResolved { median_delta_ns, .. } => median_delta_ns,
        }
    }

    /// The band, whichever variant this is.
    #[must_use]
    pub fn band_ns(&self) -> f64 {
        match *self {
            Contrast::Resolved { band_ns, .. } | Contrast::NotResolved { band_ns, .. } => band_ns,
        }
    }

    /// `true` only for [`Contrast::Resolved`].
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        matches!(self, Contrast::Resolved { .. })
    }
}

/// The propagated standard error of the difference of the two legs' medians, ns.
///
/// `SE(median) ≈ 1.2533 · σ / √n` for a large sample — the asymptotic relative efficiency of the
/// median against the mean. The two legs' SEs add in quadrature because the legs are separate
/// processes.
///
/// ⚠️ **The 1.2533 factor is exact only for a normal sample, and GPU frame times are not normal** —
/// they are right-skewed with a hard floor at the hardware quantum. For a skewed distribution the
/// median's SE is *smaller* than this, so the term is CONSERVATIVE: it widens the band rather than
/// narrowing it. That direction is the reason the approximation is acceptable here and is stated so
/// nobody later "corrects" it into something that can manufacture a win.
#[must_use]
pub fn se_floor(a: &LegSummary, b: &LegSummary) -> f64 {
    const MEDIAN_SE_FACTOR: f64 = 1.2533;
    let se = |l: &LegSummary| {
        if l.n == 0 { 0.0 } else { MEDIAN_SE_FACTOR * l.stddev_ns / (l.n as f64).sqrt() }
    };
    let (sa, sb) = (se(a), se(b));
    (sa * sa + sb * sb).sqrt()
}

/// **The comparator.**
///
/// Returns [`Contrast::Resolved`] only when every licensing condition holds AND the delta clears
/// the band. Every other outcome is a [`Contrast::NotResolved`] carrying its reason and its
/// numbers.
///
/// The refusal order is deliberate: the checks that say *"these inputs do not license a
/// comparison"* run BEFORE the band is consulted, so a caller never sees `BelowBand` on a pair of
/// legs that were never comparable — a reason that would send them to look for a bigger effect
/// instead of a valid measurement.
#[must_use]
pub fn resolve(a: &LegSummary, b: &LegSummary, floor: &Floor, twin: &Twin) -> Contrast {
    let median_delta_ns = b.median_ns - a.median_ns;
    let refuse = |reason| Contrast::NotResolved {
        reason,
        median_delta_ns,
        band_ns: 0.0,
        binding: BandTerm::Floor,
    };

    if floor.workload != a.workload || floor.workload != b.workload {
        return refuse(NotResolvedReason::FloorWorkloadMismatch);
    }
    if twin.workload != a.workload {
        return refuse(NotResolvedReason::TwinWorkloadMismatch);
    }
    if a.clock_epoch != b.clock_epoch {
        return refuse(NotResolvedReason::EpochBreak);
    }
    if !a.window_complete || !b.window_complete {
        return refuse(NotResolvedReason::WindowIncomplete);
    }
    if !a.all_measured || !b.all_measured {
        return refuse(NotResolvedReason::LabelNotMeasured);
    }

    // The four terms. `|median_a|` and not the delta: the floor is a RELATIVE claim about the
    // magnitude being measured, so scaling it by the delta would make a small delta trivially
    // decidable — the floor would shrink exactly as fast as the thing it is meant to bound.
    let terms = [
        (floor.rel * a.median_ns.abs(), BandTerm::Floor),
        (twin.ns, BandTerm::Twin),
        (se_floor(a, b), BandTerm::StandardError),
        (a.quantum_ns.max(b.quantum_ns), BandTerm::Quantum),
    ];
    let mut band = terms[0];
    for &t in &terms[1..] {
        if t.0 > band.0 {
            band = t;
        }
    }
    let (band_ns, binding) = band;

    if median_delta_ns.abs() <= band_ns {
        return Contrast::NotResolved {
            reason: NotResolvedReason::BelowBand,
            median_delta_ns,
            band_ns,
            binding,
        };
    }
    Contrast::Resolved { median_delta_ns, band_ns, binding }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(median_ns: f64, workload: WorkloadTag) -> LegSummary {
        LegSummary {
            workload,
            median_ns,
            p95_ns: median_ns * 1.05,
            stddev_ns: 0.0,
            n: 100,
            quantum_ns: 0.0,
            clock_epoch: (1, 2),
            all_measured: true,
            window_complete: true,
        }
    }

    fn floor_of(rel_all: &[f64], workload: WorkloadTag, reduction: Reduction) -> Floor {
        let (rel, rel_source_repeat) = reduction.apply(rel_all);
        Floor {
            rel,
            rel_all: rel_all.to_vec(),
            rel_source_repeat,
            workload,
            sessions: FLOOR_SESSIONS,
            repeats: FLOOR_REPEATS,
            path: PathBuf::from("<test>"),
        }
    }

    fn twin_of(ns: f64, workload: WorkloadTag) -> Twin {
        Twin { ns, rounds: 10, workload }
    }

    /// **G3a.** An A/A contrast — the same code on both legs — cannot return `Resolved`.
    #[test]
    fn an_a_a_contrast_is_never_resolved() {
        let w = WorkloadTag(7);
        let (a, b) = (leg(1000.0, w), leg(1000.0, w));
        let c = resolve(&a, &b, &floor_of(&[0.05, 0.06, 0.07], w, FLOOR_REDUCTION), &twin_of(0.0, w));
        assert!(
            matches!(c, Contrast::NotResolved { reason: NotResolvedReason::BelowBand, .. }),
            "a zero delta must be BelowBand, got {c:?}"
        );
        // And the refusal still carries its numbers.
        assert!((c.median_delta_ns() - 0.0).abs() < f64::EPSILON);
    }

    /// **G3a's reduction RED (M11).** The SAME three repetition floors and the SAME injected delta;
    /// the ONLY thing that moves is [`FLOOR_REDUCTION`]. `Max` refuses, `Min` resolves — which is
    /// the false-win machine the const exists to prevent, demonstrated rather than asserted.
    #[test]
    fn the_reduction_alone_decides_the_verdict() {
        let w = WorkloadTag(7);
        // rel_all spans 4.7 % .. 14.3 %; the delta is 8 % of leg A. `min` puts the band under it,
        // `max` puts the band over it. No other input differs between the two calls.
        let rel_all = [0.047, 0.143, 0.063];
        let (a, b) = (leg(1000.0, w), leg(1080.0, w));
        let twin = twin_of(0.0, w);

        let shipped = resolve(&a, &b, &floor_of(&rel_all, w, Reduction::Max), &twin);
        assert!(
            matches!(shipped, Contrast::NotResolved { reason: NotResolvedReason::BelowBand, .. }),
            "with Reduction::Max the delta is inside the band, got {shipped:?}"
        );

        let lucky = resolve(&a, &b, &floor_of(&rel_all, w, Reduction::Min), &twin);
        assert!(
            lucky.is_resolved(),
            "the RED: with Reduction::Min the SAME data resolves -- if this stops being true the \
             gate above no longer demonstrates anything, got {lucky:?}"
        );
        assert_eq!(FLOOR_REDUCTION, Reduction::Max, "production must ship the honest reduction");
    }

    /// **G3b.** A positive control: a delta far outside every band term resolves, and the reported
    /// delta is the real one.
    #[test]
    fn a_large_delta_resolves_with_its_own_number() {
        let w = WorkloadTag(7);
        let (a, b) = (leg(1000.0, w), leg(3000.0, w));
        let c = resolve(&a, &b, &floor_of(&[0.05, 0.06, 0.07], w, FLOOR_REDUCTION), &twin_of(0.0, w));
        match c {
            Contrast::Resolved { median_delta_ns, band_ns, binding } => {
                assert!((median_delta_ns - 2000.0).abs() < f64::EPSILON);
                assert!(band_ns > 0.0 && band_ns < 2000.0);
                assert_eq!(binding, BandTerm::Floor);
            }
            other => panic!("a 200 % delta must resolve, got {other:?}"),
        }
    }

    /// A floor measured on another workload licenses nothing, and the check runs BEFORE the band —
    /// so the caller is told the inputs were wrong, not that the effect was small.
    #[test]
    fn a_floor_from_another_workload_is_refused_before_the_band() {
        let (w, other) = (WorkloadTag(7), WorkloadTag(8));
        let (a, b) = (leg(1000.0, w), leg(9000.0, w));
        let c =
            resolve(&a, &b, &floor_of(&[0.01, 0.01, 0.01], other, FLOOR_REDUCTION), &twin_of(0.0, w));
        assert!(matches!(
            c,
            Contrast::NotResolved { reason: NotResolvedReason::FloorWorkloadMismatch, .. }
        ));
        assert!((c.band_ns() - 0.0).abs() < f64::EPSILON, "no band is computed on a refused input");
        assert!((c.median_delta_ns() - 8000.0).abs() < f64::EPSILON, "the delta is still reported");
    }

    /// Each licensing check fires on its own condition, and none of them is shadowed by another.
    #[test]
    fn every_licensing_refusal_is_reachable() {
        let w = WorkloadTag(7);
        let floor = floor_of(&[0.01, 0.01, 0.01], w, FLOOR_REDUCTION);
        let twin = twin_of(0.0, w);
        let base = leg(1000.0, w);

        let mut other_epoch = leg(9000.0, w);
        other_epoch.clock_epoch = (99, 99);
        assert!(matches!(
            resolve(&base, &other_epoch, &floor, &twin),
            Contrast::NotResolved { reason: NotResolvedReason::EpochBreak, .. }
        ));

        let mut dropped = leg(9000.0, w);
        dropped.window_complete = false;
        assert!(matches!(
            resolve(&base, &dropped, &floor, &twin),
            Contrast::NotResolved { reason: NotResolvedReason::WindowIncomplete, .. }
        ));

        let mut unmeasured = leg(9000.0, w);
        unmeasured.all_measured = false;
        assert!(matches!(
            resolve(&base, &unmeasured, &floor, &twin),
            Contrast::NotResolved { reason: NotResolvedReason::LabelNotMeasured, .. }
        ));

        assert!(matches!(
            resolve(&base, &leg(9000.0, w), &floor, &twin_of(0.0, WorkloadTag(8))),
            Contrast::NotResolved { reason: NotResolvedReason::TwinWorkloadMismatch, .. }
        ));
    }

    /// Every band term can be the binding one, so none of the four is decoration.
    #[test]
    fn each_band_term_can_bind() {
        let w = WorkloadTag(7);
        let tiny = floor_of(&[1e-9, 1e-9, 1e-9], w, FLOOR_REDUCTION);
        let (a, b) = (leg(1000.0, w), leg(1000.0, w));

        let by_twin = resolve(&a, &b, &tiny, &twin_of(500.0, w));
        assert!(matches!(by_twin, Contrast::NotResolved { binding: BandTerm::Twin, .. }));

        let mut noisy = leg(1000.0, w);
        noisy.stddev_ns = 1000.0;
        let by_se = resolve(&a, &noisy, &tiny, &twin_of(0.0, w));
        assert!(matches!(by_se, Contrast::NotResolved { binding: BandTerm::StandardError, .. }));

        let mut coarse = leg(1000.0, w);
        coarse.quantum_ns = 400.0;
        let by_quantum = resolve(&a, &coarse, &tiny, &twin_of(0.0, w));
        assert!(matches!(by_quantum, Contrast::NotResolved { binding: BandTerm::Quantum, .. }));

        let by_floor = resolve(&a, &b, &floor_of(&[0.1, 0.1, 0.1], w, FLOOR_REDUCTION), &twin_of(0.0, w));
        assert!(matches!(by_floor, Contrast::NotResolved { binding: BandTerm::Floor, .. }));
    }

    /// The session file round-trips, and `from_session_file` applies the const reduction itself.
    #[test]
    fn a_session_file_round_trips_through_the_only_constructor() {
        let dir = std::env::temp_dir().join("boyko_floor_roundtrip");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("floor_session.toml");
        let rel_all = [0.047, 0.143, 0.063];
        fs::write(&path, Floor::render_session_file(&rel_all, WorkloadTag(1234), FLOOR_SESSIONS))
            .expect("write the session file");

        let f = Floor::from_session_file(&path).expect("the writer's own output must parse");
        assert_eq!(f.rel_all(), &rel_all, "all three are carried, never averaged");
        assert!((f.rel() - 0.143).abs() < f64::EPSILON, "the const reduction is Max");
        assert_eq!(f.rel_source_repeat(), 1, "and it names which repetition supplied it");
        assert_eq!(f.workload().as_u64(), 1234);
        let _ = fs::remove_file(&path);
    }

    /// A file recording a different protocol is refused rather than read: a floor taken over 3
    /// sessions is not this build's floor, and reading it would be the "different instrument"
    /// failure with the numbers looking fine.
    #[test]
    fn a_session_file_from_another_protocol_is_refused() {
        let dir = std::env::temp_dir().join("boyko_floor_roundtrip");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("floor_wrong_protocol.toml");
        fs::write(&path, Floor::render_session_file(&[0.05, 0.06, 0.07], WorkloadTag(1), 3))
            .expect("write");
        assert!(matches!(
            Floor::from_session_file(&path),
            Err(FloorError::ProtocolMismatch { found: (3, 3), expected: (7, 3) })
        ));
        let _ = fs::remove_file(&path);
    }

    /// A tag is over a SET: two callers naming the same zones in different orders agree, and one
    /// naming a different set does not.
    #[test]
    fn the_workload_tag_is_over_the_zone_set() {
        use super::super::artifact::{ArtifactHeader, LabelCensus};
        let art = Artifact {
            header: ArtifactHeader {
                schema_version: super::super::artifact::ARTIFACT_SCHEMA_VERSION,
                session_lo: 1,
                session_hi: 2,
                run_token: "t".into(),
                workload_tag: "vb_mesh#abcd1234".into(),
                content_tag: "n14_kronecker".into(),
                instrument: super::super::artifact::Instrument::Live,
                precision_decimals: 1,
                regimes: "-".into(),
                modes: "-".into(),
                regime_n_distinct: 0,
            },
            zones: Vec::new(),
            census: LabelCensus::default(),
            losses: Vec::new(),
        };
        assert_eq!(WorkloadTag::of(&art, &[2, 0, 1]), WorkloadTag::of(&art, &[0, 1, 2]));
        assert_ne!(WorkloadTag::of(&art, &[0, 1, 2]), WorkloadTag::of(&art, &[0, 1, 2, 9]));
    }
}
