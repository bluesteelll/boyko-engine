//! Profiling rung 7 — the measurement artifact: the file that replaces the stdout channel.
//!
//! The corpus specifies that rung 7 deletes the printed measurement lines and migrates their
//! consumers to *"the artifact"*, and it specifies almost nothing about the artifact itself.
//! MEASURED before this file was written: a search for TOML table syntax across the whole
//! profiling corpus plus `SEAM.md` returns **zero** — the format was never written down. What the
//! corpus does pin is `schema_version` on a *"flat TOML"* file, `p95_lo`/`p95_hi`, the measured
//! quantum, `sum = NOT_VALID (mixed stage)`, `cpu_gpu_offset = UNCORRELATED`, *"per-zone rows"* and
//! *"the artifact's label census"*. Everything below that is not one of those is a decision made
//! here, and each one is stated with what it costs to get wrong.
//!
//! # Decision 1 — ONE DECIMAL PLACE, and the reason it is NOT the one I first wrote
//!
//! [`PRECISION_DECIMALS`] is `1`: the artifact carries every nanosecond figure rounded to a tenth,
//! because the channel it replaces did.
//!
//! **The first justification for this was wrong, and insisting on a RED is what found it.**
//! `vg_occ_split_timing.rs:916` reconstructs the GPU tick lattice by GCD over tenths —
//! `(v * 10.0).round()` — and the obvious fear is that a wider file collapses that GCD. MEASURED,
//! by injecting full precision into the writer and running the gate: **it cannot.** The consumer's
//! own `.round()` *is* a rounding to tenths, so it absorbs whatever extra digits the file carries;
//! across `128.0`, `268.8`, `163.2`, `128.04`, `128.06`, `12.85` and `1234.567` the reconstructed
//! value is identical either way. The 32× under-statement that file's doc measures is about
//! choosing the FLOOR term as `period × 1 tick`, which is a different decision entirely.
//!
//! So the honest reasons for one decimal are smaller ones, and they are enough: the figures are
//! **directly comparable with the printed lines** the migration replaces, which is what makes the
//! next step's A/B possible at all; the file stays small; and
//! [`ArtifactHeader::precision_decimals`] states the choice *in the file*, so a future consumer
//! that does NOT re-round has a number to read instead of a convention to inherit.
//!
//! The gate says exactly this much and no more — see
//! `tests/profiling_artifact_roundtrip.rs`'s precision clause, which discloses that its widening
//! RED is not producible against today's consumers rather than pretending otherwise.
//!
//! # Decision 2/3 — one process, one file, TRUNCATED at open
//!
//! `vg_decidability_floor.rs` spawns **42 sequential children**, so a fixed shared path is a
//! stale-read generator. The parent chooses the path (it is the only party that knows which child
//! it is reading), the writer truncates at open, and rows append within the process. That makes
//! "one file = one process" structural rather than conventional, and it is the reading under which
//! the corpus's own verb `append_artifact` stays honest: appending happens *within* a sitting.
//!
//! # Decision 4 — the staleness discriminator is a PARENT-SUPPLIED RUN TOKEN, because the two
//! # fields `G24` names cannot do the job
//!
//! `G24`'s reverse RED requires a reader to refuse a **stale** artifact on a header mismatch, and
//! names `build_hash` and `SessionId`. Measured against the tree:
//!
//! * **`build_hash` does not exist.** `crates/boyko_diag/` contains `Cargo.toml` and `src/` and
//!   **no `build.rs` at all**; `BUILD_HASH` appears nowhere in the workspace. It is a planned rung-0
//!   artifact that never landed. It is therefore **not** a field of this file: a header field that
//!   is always absent is indistinguishable from one that is broken, and adding it when it exists is
//!   one line.
//! * **`SessionId` exists** (`boyko_diag::clock::session_id`, 128 bits, minted once per process) and
//!   **cannot be predicted by a parent**, because it is minted *inside the child*. A parent can only
//!   compare it against a value the child already told it — which is the thing staleness would have
//!   corrupted.
//!
//! So the only field that can catch a stale read *within one run* is one the **parent chooses before
//! the child starts**: [`ArtifactHeader::run_token`]. The child stamps what it was given; the reader
//! refuses anything else. `SessionId` is still carried — it is what proves two files came from one
//! process, which is its stated job — but it is not the discriminator.
//!
//! # Decision 5 — the workload tag is TWO fields, and an UNDECLARED one is not a floor
//!
//! ⚠️ **First, a correction to what this section used to say.** It read *"`resolve` refuses a
//! `Floor` whose workload tag does not match"* as if that were shipped code. MEASURED: `Floor`,
//! `resolve`, `FloorWorkloadMismatch` and `NotResolved` appear **only in the corpus documents** —
//! `rg` over `crates/` returns nothing for any of them. They are rung 8's content and are unwritten.
//! So the tag is not feeding a live comparator today; it is the INPUT to one that does not exist
//! yet, which is exactly why what it must name has to be settled before rung 7b publishes a floor
//! that later rungs will cite.
//!
//! **The hole this closes, measured.** The tag was `format!("{path:?}_{legs:?}")`.
//! `vg_decidability_floor.rs` measures the floor with 42 processes across **two configurations** —
//! `BOYKO_VB_FROXEL_FORCE_OFF` set and unset — and neither `path` nor `legs` changes between them,
//! because `froxel_light_cull` is a *different field of the same struct*. The same bench sweeps
//! `N_ps ∈ {8, 64, 256, 512}`, which the engine cannot see at all: the test's own setup spawns
//! those lights. Two things a floor must never be shared across, sharing one tag.
//!
//! **So the tag is split by who can know it, and the split is not cosmetic.**
//!
//! * [`ArtifactHeader::workload_tag`] — **DERIVED, unforgeable.** Built by [`config_tag`] from the
//!   WHOLE of `ResolvedRenderPath`: a readable `path_legs` prefix a human can grep, plus a hash over
//!   the struct's every field. Exhaustive rather than a hand-picked subset **because a hand-picked
//!   subset is how `froxel_light_cull` was left out in the first place** — the mistake is not that
//!   the wrong field was chosen, it is that fields were chosen. A field added to
//!   `ResolvedRenderPath` therefore invalidates prior floors, deliberately: floors on this box drift
//!   on a timescale shorter than the gap between two measurements anyway
//!   (`vg_decidability_floor.rs`'s own finding), so re-measuring is cheap and a wrong bound is not.
//! * [`ArtifactHeader::content_tag`] — **DECLARED, and empty is a real state.** Light count, scene,
//!   rig: nothing in the engine can derive them. The measuring test declares them
//!   (`BOYKO_PROFILE_WORKLOAD`, set in the spawner's own code where the value already lives, not in
//!   an operator's shell where it can be forgotten per-run).
//!
//! **An artifact whose `content_tag` is empty CANNOT serve as a floor** — [`Artifact::floor_source`]
//! refuses it with [`ArtifactError::UndeclaredContent`]. Owner's call, taken as the strict option of
//! three. The reason it is enforced here rather than promised to rung 8: this campaign has already
//! measured that a clause whose subject does not exist yet is a promise, not a gate, and that
//! **absence reads as a passing state** unless something refuses it. An undeclared content tag is a
//! value nothing can make move.
//!
//! It is a SEPARATE field rather than an empty suffix on the derived one for the same reason: a
//! composite string cannot distinguish *"declared nothing"* from *"declared, and it happened to be
//! short"*, and that distinction is the whole of the refusal.
//!
//! The check is scoped to *being a floor*. An artifact without a content tag is still perfectly
//! readable and still gates liveness — `gbuffer_zone_port_gate.rs`'s census-agreement clause reads
//! one and has no business declaring a workload.
//!
//! # Decision 7 — the REGIME is a census, because it is a per-window observation
//!
//! `vg_occ_split_timing.rs` requires every worker's occlusion regime and rejects one whose
//! `n_distinct != 1` *"rather than averaging two regimes into one number"*. Nothing already in this
//! header can answer that. [`ArtifactHeader::workload_tag`] is derived from the boot-frozen
//! `ResolvedRenderPath`, and VG R3 rung P4-4 made the regime a **live `Resource`** — it can change
//! between frames of one window, so no boot-time value can see it.
//!
//! So it is shaped like [`LabelCensus`]: the SET observed across the window plus its cardinality.
//! *"How many regimes did this window time?"* and *"how many pairs came back `Measured`?"* are the
//! same kind of question, and the answer to both is an observation over frames rather than a
//! property of the configuration.
//!
//! **Recorded, never asserted.** The printed channel's own line says why: `n_distinct > 1` *"is
//! printed, never asserted, because a constancy assertion would have to hold on hosts this
//! repository does not own"*. The consumer rejects; the file reports.
//!
//! # Decision 6 — a declined instrument is a HEADER FIELD, not a line on stderr
//!
//! Three consumers key their third outcome ("neither green nor red") on the `eprintln!` that says
//! the device cannot serve timestamps. If that stayed on stderr, *"the channel became a file"* would
//! be false for exactly the case where a reader most needs to know what it is looking at.
//! [`Instrument::NoTimestamps`] puts it in the header.
//!
//! # Format
//!
//! Flat TOML: bare `key = value` lines for the header and the census, then one `[[zone]]` block per
//! zone — the *"per-zone rows"* the corpus names, and the only table in the file. Emitted and parsed
//! by hand: this crate ships **zero third-party dependencies** and a serializer is not a reason to
//! break that.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

/// The artifact's format version. A reader refuses anything else **before parsing a row**.
///
/// `2` since rung 7c's tag split: a v1 file carries no `content_tag`, and reading one as if the
/// field were merely empty would hand a floor exactly the "declared nothing" state
/// [`Artifact::floor_source`] exists to refuse.
pub const ARTIFACT_SCHEMA_VERSION: u32 = 7;

/// The **derived, unforgeable** half of a workload tag: everything about the boot-resolved
/// configuration that the engine itself knows.
///
/// `"<path>_<legs>#<hash>"` — a prefix a human can read and grep, and eight hex digits of FNV-1a
/// over the `Debug` rendering of the WHOLE [`ResolvedRenderPath`]. The hash covers every field
/// rather than a chosen few; see the module doc's Decision 5 for why choosing is the bug.
///
/// Hashing `Debug` rather than the struct's bytes is deliberate: `ResolvedRenderPath` is `repr(C)`
/// and `Copy`, but reading it as bytes would fold in padding this code does not control, and a tag
/// that changes with uninitialised padding is worse than no tag.
#[must_use]
pub fn config_tag(resolved: &boyko_render::ResolvedRenderPath) -> String {
    // FNV-1a/64. Chosen because it is four lines and has no dependency; this is a discriminator,
    // not a security primitive, and collisions between two configurations of one struct are not a
    // threat model.
    let rendered = format!("{resolved:?}");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in rendered.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:?}_{:?}#{:08x}", resolved.path, resolved.legs, (h >> 32) as u32).to_lowercase()
}

/// Decimal places every nanosecond figure carries. See the module doc's Decision 1 — this is a
/// property of the instrument, not a formatting preference.
pub const PRECISION_DECIMALS: u8 = 1;

/// Whether the run that wrote this file had a working timestamp instrument.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Instrument {
    /// Timestamps were usable; the rows below are measurements.
    Live,
    /// The device declined timestamps, so no bracket could be timed. The rows (if any) carry no
    /// durations, and a consumer must report its third outcome rather than green or red.
    NoTimestamps,
}

impl Instrument {
    /// The token written into the file.
    fn as_str(self) -> &'static str {
        match self {
            Instrument::Live => "live",
            Instrument::NoTimestamps => "no_timestamps",
        }
    }

    fn parse(s: &str) -> Option<Instrument> {
        match s {
            "live" => Some(Instrument::Live),
            "no_timestamps" => Some(Instrument::NoTimestamps),
            _ => None,
        }
    }
}

/// A recorded pair's outcome — the `GpuZoneRecorder` 2×2 label, carried into the file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoneLabel {
    /// Bracketed and available: the duration is a measurement.
    Measured,
    /// The recorder never bracketed this zone in this window.
    NotBracketed,
    /// Bracketed, never available.
    Lost,
    /// A begin with no end.
    Torn,
}

impl ZoneLabel {
    fn as_str(self) -> &'static str {
        match self {
            ZoneLabel::Measured => "measured",
            ZoneLabel::NotBracketed => "not_bracketed",
            ZoneLabel::Lost => "lost",
            ZoneLabel::Torn => "torn",
        }
    }

    fn parse(s: &str) -> Option<ZoneLabel> {
        match s {
            "measured" => Some(ZoneLabel::Measured),
            "not_bracketed" => Some(ZoneLabel::NotBracketed),
            "lost" => Some(ZoneLabel::Lost),
            "torn" => Some(ZoneLabel::Torn),
            _ => None,
        }
    }
}

/// The file's header. Every field is checked or carried for a reason stated in the module doc.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArtifactHeader {
    /// [`ARTIFACT_SCHEMA_VERSION`] at write time.
    pub schema_version: u32,
    /// `boyko_diag::clock::session_id`'s low half — proves two files came from one process.
    pub session_lo: u64,
    /// Its high half.
    pub session_hi: u64,
    /// **The staleness discriminator.** Chosen by the parent BEFORE the child starts; see the module
    /// doc's Decision 4. Empty when no parent supplied one, in which case a reader that passes a
    /// non-empty expectation refuses — an unstamped file cannot prove it is this run's.
    pub run_token: String,
    /// The DERIVED half of what was measured — [`config_tag`] over the whole boot-resolved path.
    /// Cannot be forged by a caller, and covers every configuration bit the engine knows.
    pub workload_tag: String,
    /// The DECLARED half — the content the engine cannot see (light count, scene, rig), named by
    /// whoever measured. **Empty means "nobody declared"**, which is a state
    /// [`Artifact::floor_source`] refuses rather than a short string. See the module doc's
    /// Decision 5.
    pub content_tag: String,
    /// Whether the timestamp instrument was alive.
    pub instrument: Instrument,
    /// [`PRECISION_DECIMALS`] at write time, stated so a reader never has to assume it.
    pub precision_decimals: u8,
    /// **The regime census** — the SET of distinct occlusion-force words the window observed, in
    /// the variants' own order, or `-` for none. See the module doc's Decision 7.
    pub regimes: String,
    /// The same for the occlusion MODE words.
    pub modes: String,
    /// **Whether the allocation-counting shim was installed** — profiling rung 8's
    /// `profiling-alloc`, and the corpus's *"its perturbation is stated in the artifact when on"*.
    ///
    /// The FLAG and not only the counts, for the reason the label census carries labels: a zero
    /// count under `false` means this build has no counter, and under `true` it means the run
    /// allocated nothing. One is a claim about the build and the other about the engine.
    ///
    /// ⚠️ **`true` marks the whole artifact as a DIAGNOSTIC-MODE reading.** The shim adds two
    /// atomic read-modify-writes per allocation, process-wide; its timings are not comparable with
    /// an unarmed run's, and `WorkloadTag` does not cover it, so a floor and a leg that disagree
    /// here would compare as if they agreed.
    pub alloc_shim: bool,
    /// Allocations since process start, or `0` when [`Self::alloc_shim`] is `false`.
    pub alloc_allocs: u64,
    /// Deallocations, same condition.
    pub alloc_deallocs: u64,
    /// Bytes REQUESTED by those allocations — not what the allocator reserved, which this shim
    /// cannot see.
    pub alloc_bytes: u64,
    /// **The present mode this boot RESOLVED to** — profiling rung 8, D12. The wire word from
    /// `PresentModeConfig::as_str`.
    ///
    /// The RESOLVED mode, never the requested one: only `fifo` is spec-guaranteed, and a file that
    /// recorded what was asked for would attribute a refresh-bounded frame time to a tearing
    /// present. It is here because a frame's wall clock means different things under different
    /// modes — under `fifo` it is bounded below by the refresh interval, so a GPU regression can be
    /// entirely invisible in it — and a reader comparing two artifacts has to be able to see that
    /// they are not comparable.
    pub present_mode: String,
    /// How many distinct regimes the window observed. **Recorded, never asserted**: a consumer that
    /// needs one regime per capture rejects a window with more, which is its rule and not this
    /// file's — a constancy assertion here would have to hold on hosts this repository does not own.
    pub regime_n_distinct: u32,
}

/// One zone's window, as the reducer produced it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ZoneRow {
    /// The zone id — `family base + pass slot` for the ported GPU brackets.
    pub zone: u16,
    /// What the recorder said about this zone's pairs in this window.
    pub label: ZoneLabel,
    /// Frames folded into the figures below.
    pub n: u32,
    /// Median duration, ns.
    pub median_ns: f64,
    /// Mean duration, ns.
    pub mean_ns: f64,
    /// 95th percentile duration, ns.
    pub p95_ns: f64,
    /// Population standard deviation of the durations, ns — schema 4, added for rung 8's
    /// `se_floor` band term. MEASURED rather than recovered from `p95 - median`, which would have
    /// assumed a normality GPU frame times do not have; see [`super::reduce::stats_ns`].
    pub stddev_ns: f64,
    /// Offset of this zone's begin from the window's base, ns.
    pub begin_off_ns: f64,
    /// Offset of its end, ns. Carried rather than derived: `begin + median` is not a time any frame
    /// had, and the record-order gates read this directly.
    pub end_off_ns: f64,
}

/// How many pairs came back under each label — the corpus's *"artifact's label census"*, and the
/// witness that replaces "a printed line existed" as a liveness proof.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LabelCensus {
    /// Pairs bracketed and available.
    pub measured: u32,
    /// Pairs the recorder never bracketed.
    pub not_bracketed: u32,
    /// Pairs bracketed whose results never arrived.
    pub lost: u32,
    /// Pairs opened and never closed.
    pub torn: u32,
}

/// One drop class the window observed, with what it cost — **profiling rung 8, `G4c`**.
///
/// Only NON-ZERO classes get a row. A class with no drops is absent rather than present-and-zero,
/// so a reader scanning the file sees exactly the classes that happened; the eight-word vocabulary
/// is in `boyko_diag::loss::LossClass` and is not repeated here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LossRow {
    /// The class's wire word — `LossClass::as_str`.
    pub class: String,
    /// How many events were lost.
    pub count: u64,
    /// Their payload cost, where the class has one. `0` for classes that do not.
    pub bytes: u64,
}

/// A whole artifact: header, per-zone rows, label census, drop census.
#[derive(Clone, PartialEq, Debug)]
pub struct Artifact {
    /// The header, checked before any row is parsed.
    pub header: ArtifactHeader,
    /// One row per zone.
    pub zones: Vec<ZoneRow>,
    /// The label census for the same window.
    pub census: LabelCensus,
    /// **Every non-zero drop class this process accrued, with its count** — `G4c`'s clause: the
    /// loss has to reach the reader, not only the counter.
    ///
    /// Process-wide and not window-scoped, and that is stated rather than glossed: `boyko_diag`'s
    /// cells are monotone totals for the process, and the artifact is written once at the end of a
    /// measured run. A run that measured two windows would attribute both windows' drops to the
    /// file it wrote; no caller does that today, and the day one does, the fix is a
    /// `LossSeen` snapshot at window open — not a re-interpretation of this field.
    pub losses: Vec<LossRow>,
}

/// Why a read refused.
#[derive(Debug)]
pub enum ArtifactError {
    /// The file could not be opened or read.
    Io(io::Error),
    /// A required header key was missing or unparseable.
    BadHeader(&'static str),
    /// The file's schema is not this build's. **Refused before any row is parsed.**
    SchemaMismatch {
        /// What the file said.
        found: u32,
        /// What this build expects.
        expected: u32,
    },
    /// The file was not stamped with the run token the reader was told to expect — a STALE artifact.
    /// **Refused before any row is parsed**, which is `G24`'s reverse RED.
    TokenMismatch {
        /// The token in the file.
        found: String,
        /// The token the reader required.
        expected: String,
    },
    /// A row was malformed.
    Malformed {
        /// 1-based line number.
        line: usize,
        /// What was wrong.
        why: &'static str,
    },
    /// The file is well-formed but its `content_tag` is EMPTY, so it does not say what workload it
    /// measured — and a floor is a property of the workload as much as of the box. Returned only by
    /// [`Artifact::floor_source`]; every other reader is unaffected. See the module doc's
    /// Decision 5.
    UndeclaredContent {
        /// The derived half, which the engine always knows — quoted so the message names the file
        /// rather than describing a category.
        workload_tag: String,
    },
}

impl core::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ArtifactError::Io(e) => write!(f, "artifact io: {e}"),
            ArtifactError::BadHeader(k) => write!(f, "artifact header: missing or bad `{k}`"),
            ArtifactError::SchemaMismatch { found, expected } => write!(
                f,
                "artifact schema {found} is not this build's {expected}; refused before parsing any row"
            ),
            ArtifactError::TokenMismatch { found, expected } => write!(
                f,
                "artifact run token {found:?} is not the expected {expected:?} — this file is from \
                 another run (STALE); refused before parsing any row"
            ),
            ArtifactError::Malformed { line, why } => {
                write!(f, "artifact line {line}: {why}")
            }
            ArtifactError::UndeclaredContent { workload_tag } => write!(
                f,
                "artifact {workload_tag:?} declares no content_tag, so it cannot serve as a FLOOR:                  a floor bounds the workload it was measured on, and this file does not say which                  one that was. The measuring test must set BOYKO_PROFILE_WORKLOAD (light count,                  scene, rig) where it spawns its children"
            ),
        }
    }
}

impl std::error::Error for ArtifactError {}

/// Formats one nanosecond figure at the instrument's precision. The ONE place that decides it.
fn ns(v: f64) -> String {
    format!("{v:.*}", PRECISION_DECIMALS as usize)
}

impl Artifact {
    /// Renders the artifact as flat TOML.
    ///
    /// Separated from [`Self::write`] so a test can assert on the text without touching a
    /// filesystem, and so the writer has exactly one rendering.
    #[must_use]
    pub fn render(&self) -> String {
        let mut s = String::with_capacity(256 + self.zones.len() * 128);
        let h = &self.header;
        let _ = writeln!(s, "schema_version = {}", h.schema_version);
        let _ = writeln!(s, "session_lo = {}", h.session_lo);
        let _ = writeln!(s, "session_hi = {}", h.session_hi);
        let _ = writeln!(s, "run_token = \"{}\"", h.run_token);
        let _ = writeln!(s, "workload_tag = \"{}\"", h.workload_tag);
        // Written even when empty. An ABSENT key would make "nobody declared" and "an older
        // writer" the same observation, and the refusal below has to tell them apart.
        let _ = writeln!(s, "content_tag = \"{}\"", h.content_tag);
        let _ = writeln!(s, "instrument = \"{}\"", h.instrument.as_str());
        let _ = writeln!(s, "precision_decimals = {}", h.precision_decimals);
        let _ = writeln!(s, "regimes = \"{}\"", h.regimes);
        let _ = writeln!(s, "modes = \"{}\"", h.modes);
        let _ = writeln!(s, "regime_n_distinct = {}", h.regime_n_distinct);
        let _ = writeln!(s, "present_mode = \"{}\"", h.present_mode);
        let _ = writeln!(s, "alloc_shim = {}", h.alloc_shim);
        let _ = writeln!(s, "alloc_allocs = {}", h.alloc_allocs);
        let _ = writeln!(s, "alloc_deallocs = {}", h.alloc_deallocs);
        let _ = writeln!(s, "alloc_bytes = {}", h.alloc_bytes);
        let c = &self.census;
        let _ = writeln!(s, "census_measured = {}", c.measured);
        let _ = writeln!(s, "census_not_bracketed = {}", c.not_bracketed);
        let _ = writeln!(s, "census_lost = {}", c.lost);
        let _ = writeln!(s, "census_torn = {}", c.torn);
        for l in &self.losses {
            let _ = writeln!(s);
            let _ = writeln!(s, "[[loss]]");
            let _ = writeln!(s, "class = \"{}\"", l.class);
            let _ = writeln!(s, "count = {}", l.count);
            let _ = writeln!(s, "bytes = {}", l.bytes);
        }
        for z in &self.zones {
            let _ = writeln!(s, "\n[[zone]]");
            let _ = writeln!(s, "id = {}", z.zone);
            let _ = writeln!(s, "label = \"{}\"", z.label.as_str());
            let _ = writeln!(s, "n = {}", z.n);
            let _ = writeln!(s, "median_ns = {}", ns(z.median_ns));
            let _ = writeln!(s, "mean_ns = {}", ns(z.mean_ns));
            let _ = writeln!(s, "p95_ns = {}", ns(z.p95_ns));
            let _ = writeln!(s, "stddev_ns = {}", ns(z.stddev_ns));
            let _ = writeln!(s, "begin_off_ns = {}", ns(z.begin_off_ns));
            let _ = writeln!(s, "end_off_ns = {}", ns(z.end_off_ns));
        }
        s
    }

    /// **`G4c`'s producer** — reads `boyko_diag`'s process-wide loss cells and returns one row per
    /// NON-ZERO class.
    ///
    /// # Why it reads every row, and why zero classes are absent
    ///
    /// `boyko_diag` keeps one cell per (lane, class); the losses this artifact reports can be
    /// recorded from any thread that folded a window, so summing across every lane row is the only
    /// reading that cannot miss one. A class with no drops gets NO ROW rather than a row of zeros:
    /// a reader scanning the file then sees exactly what happened, and *"the file says nothing
    /// about `Rotation`"* and *"the file says `Rotation` was zero"* are the same statement here,
    /// unlike the header's `content_tag` where they are not.
    ///
    /// **This is a TOTAL, not a window delta**, and [`Self::losses`]' own doc says what that costs.
    /// It uses the raw cell rather than `delta_since` deliberately: a delta needs a `LossSeen`
    /// snapshot taken at window open, and there is no window-open hook to take it in. Adding one
    /// would be the correct fix the day a process writes two artifacts; inventing a snapshot here
    /// would make the number look window-scoped while being process-scoped.
    #[must_use]
    pub fn collect_losses() -> Vec<LossRow> {
        use boyko_diag::loss::{LOSS_ROW_COUNT, LossClass, cell_at_row};
        let mut out = Vec::new();
        for class in LossClass::ALL {
            let mut count = 0u64;
            let mut bytes = 0u64;
            for row in 0..LOSS_ROW_COUNT {
                let c = cell_at_row(row, class);
                count += c.count();
                bytes += c.bytes();
            }
            if count != 0 || bytes != 0 {
                out.push(LossRow { class: class.as_str().to_owned(), count, bytes });
            }
        }
        out
    }

    /// The count this artifact reports for one class, or `0` when it reports no row for it.
    ///
    /// The accessor exists so a consumer never has to decide what an ABSENT row means: it means
    /// zero, stated once here rather than at every reader.
    #[must_use]
    pub fn loss_count(&self, class: &str) -> u64 {
        self.losses.iter().find(|l| l.class == class).map_or(0, |l| l.count)
    }

    /// **The floor gate.** Returns this artifact only if it says what workload it measured.
    ///
    /// Rung 8's `Floor` does not exist yet; this is the refusal it will call, shipped now and gated
    /// now, because a clause whose subject is unwritten is a promise rather than a gate — a thing
    /// this campaign has measured more than once. `resolve` comparing tags is a SEPARATE check and
    /// stays rung 8's: this one asks only whether there is anything to compare.
    ///
    /// Deliberately NOT folded into [`Self::read`]. Most readers of an artifact are not looking for
    /// a floor — `gbuffer_zone_port_gate.rs` reads one to check a label census — and refusing them
    /// would make the strict rule cost work it was never meant to gate.
    ///
    /// # Errors
    ///
    /// [`ArtifactError::UndeclaredContent`] when `content_tag` is empty or blank.
    pub fn floor_source(&self) -> Result<&Artifact, ArtifactError> {
        // Trimmed, so a tag of spaces is the same refusal as no tag at all: whitespace declares no
        // more about a workload than emptiness does, and the two must not be different outcomes.
        if self.header.content_tag.trim().is_empty() {
            return Err(ArtifactError::UndeclaredContent {
                workload_tag: self.header.workload_tag.clone(),
            });
        }
        Ok(self)
    }

    /// Writes the artifact to `path`, **truncating** — one process, one file (Decision 2/3).
    ///
    /// # Errors
    ///
    /// Propagates any filesystem error.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        fs::write(path, self.render())
    }

    /// Reads and validates an artifact.
    ///
    /// `expect_run_token` is the token the caller chose for the child that should have written this
    /// file. A non-empty expectation that does not match the file's is
    /// [`ArtifactError::TokenMismatch`] — **returned before a single row is parsed**, which is what
    /// `G24`'s reverse RED asserts. An empty expectation waives the check, for the one caller that
    /// genuinely does not know (an operator reading a file by hand).
    ///
    /// # Errors
    ///
    /// [`ArtifactError`] for io failure, a missing or malformed header, a schema or token mismatch,
    /// or a malformed row.
    pub fn read(path: &Path, expect_run_token: &str) -> Result<Artifact, ArtifactError> {
        let text = fs::read_to_string(path).map_err(ArtifactError::Io)?;
        Artifact::parse(&text, expect_run_token)
    }

    /// [`Self::read`]'s body, on text already in memory.
    ///
    /// # Errors
    ///
    /// As [`Self::read`], minus the io case.
    pub fn parse(text: &str, expect_run_token: &str) -> Result<Artifact, ArtifactError> {
        // ---- header first, and the refusals BEFORE any row is looked at ----------------------
        //
        // The ordering is the gate's subject, not an optimisation: a reader that parsed rows and
        // then checked the header would have already produced the numbers it was supposed to
        // refuse, and every caller that ignored the error would use them.
        let mut schema_version: Option<u32> = None;
        let mut run_token: Option<String> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line == "[[zone]]" {
                break;
            }
            let Some((k, v)) = split_kv(line) else { continue };
            match k {
                "schema_version" => {
                    schema_version = Some(v.parse().map_err(|_| ArtifactError::Malformed {
                        line: i + 1,
                        why: "schema_version is not an integer",
                    })?);
                }
                "run_token" => run_token = Some(unquote(v).to_owned()),
                _ => {}
            }
        }
        let schema_version = schema_version.ok_or(ArtifactError::BadHeader("schema_version"))?;
        if schema_version != ARTIFACT_SCHEMA_VERSION {
            return Err(ArtifactError::SchemaMismatch {
                found: schema_version,
                expected: ARTIFACT_SCHEMA_VERSION,
            });
        }
        let run_token = run_token.ok_or(ArtifactError::BadHeader("run_token"))?;
        if !expect_run_token.is_empty() && run_token != expect_run_token {
            return Err(ArtifactError::TokenMismatch {
                found: run_token,
                expected: expect_run_token.to_owned(),
            });
        }

        // ---- the rest ------------------------------------------------------------------------
        let mut session_lo = None;
        let mut session_hi = None;
        let mut workload_tag = None;
        let mut content_tag = None;
        let mut regimes = None;
        let mut modes = None;
        let mut regime_n_distinct = None;
        let mut instrument = None;
        let mut precision_decimals = None;
        let mut present_mode: Option<String> = None;
        let mut alloc_shim: Option<bool> = None;
        let mut alloc_allocs: Option<u64> = None;
        let mut alloc_deallocs: Option<u64> = None;
        let mut alloc_bytes: Option<u64> = None;
        let mut census = LabelCensus::default();
        let mut zones: Vec<ZoneRow> = Vec::new();
        let mut cur: Option<PartialZone> = None;
        let mut losses: Vec<LossRow> = Vec::new();
        let mut cur_loss: Option<PartialLoss> = None;

        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[loss]]" {
                if let Some(p) = cur_loss.take() {
                    losses.push(p.finish(i + 1)?);
                }
                cur_loss = Some(PartialLoss::default());
                continue;
            }
            if line == "[[zone]]" {
                // The loss blocks come first in the rendered order, so the first `[[zone]]` closes
                // whichever loss block was open. Written as a close-on-transition rather than as
                // two passes because a second pass would have to agree with this one about where
                // the sections are, and two parsers of one file is how they come to disagree.
                if let Some(p) = cur_loss.take() {
                    losses.push(p.finish(i + 1)?);
                }
                if let Some(p) = cur.take() {
                    zones.push(p.finish(i + 1)?);
                }
                cur = Some(PartialZone::default());
                continue;
            }
            if let Some(l) = cur_loss.as_mut() {
                let bad = |why| ArtifactError::Malformed { line: i + 1, why };
                let Some((k, v)) = split_kv(line) else {
                    return Err(bad("not a `key = value` line"));
                };
                match k {
                    "class" => l.class = Some(unquote(v).to_owned()),
                    "count" => l.count = Some(v.parse().map_err(|_| bad("count is not a u64"))?),
                    "bytes" => l.bytes = Some(v.parse().map_err(|_| bad("bytes is not a u64"))?),
                    _ => {}
                }
                continue;
            }
            let Some((k, v)) = split_kv(line) else {
                return Err(ArtifactError::Malformed { line: i + 1, why: "not a `key = value` line" });
            };
            let bad = |why| ArtifactError::Malformed { line: i + 1, why };
            if let Some(z) = cur.as_mut() {
                match k {
                    "id" => z.zone = Some(v.parse().map_err(|_| bad("id is not a u16"))?),
                    "label" => {
                        z.label = Some(
                            ZoneLabel::parse(unquote(v)).ok_or_else(|| bad("unknown zone label"))?,
                        );
                    }
                    "n" => z.n = Some(v.parse().map_err(|_| bad("n is not a u32"))?),
                    "median_ns" => z.median = Some(v.parse().map_err(|_| bad("median_ns"))?),
                    "mean_ns" => z.mean = Some(v.parse().map_err(|_| bad("mean_ns"))?),
                    "p95_ns" => z.p95 = Some(v.parse().map_err(|_| bad("p95_ns"))?),
                    "stddev_ns" => z.stddev = Some(v.parse().map_err(|_| bad("stddev_ns"))?),
                    "begin_off_ns" => z.begin = Some(v.parse().map_err(|_| bad("begin_off_ns"))?),
                    "end_off_ns" => z.end = Some(v.parse().map_err(|_| bad("end_off_ns"))?),
                    _ => {}
                }
                continue;
            }
            match k {
                "session_lo" => session_lo = Some(v.parse().map_err(|_| bad("session_lo"))?),
                "session_hi" => session_hi = Some(v.parse().map_err(|_| bad("session_hi"))?),
                "workload_tag" => workload_tag = Some(unquote(v).to_owned()),
                "content_tag" => content_tag = Some(unquote(v).to_owned()),
                "regimes" => regimes = Some(unquote(v).to_owned()),
                "modes" => modes = Some(unquote(v).to_owned()),
                "regime_n_distinct" => regime_n_distinct = v.trim().parse().ok(),
                "present_mode" => present_mode = Some(unquote(v).to_owned()),
                "alloc_shim" => alloc_shim = Some(v.trim() == "true"),
                "alloc_allocs" => alloc_allocs = v.trim().parse().ok(),
                "alloc_deallocs" => alloc_deallocs = v.trim().parse().ok(),
                "alloc_bytes" => alloc_bytes = v.trim().parse().ok(),
                "instrument" => {
                    instrument =
                        Some(Instrument::parse(unquote(v)).ok_or_else(|| bad("unknown instrument"))?);
                }
                "precision_decimals" => {
                    precision_decimals = Some(v.parse().map_err(|_| bad("precision_decimals"))?);
                }
                "census_measured" => census.measured = v.parse().map_err(|_| bad("census_measured"))?,
                "census_not_bracketed" => {
                    census.not_bracketed = v.parse().map_err(|_| bad("census_not_bracketed"))?;
                }
                "census_lost" => census.lost = v.parse().map_err(|_| bad("census_lost"))?,
                "census_torn" => census.torn = v.parse().map_err(|_| bad("census_torn"))?,
                _ => {}
            }
        }
        if let Some(p) = cur.take() {
            zones.push(p.finish(text.lines().count())?);
        }

        if let Some(p) = cur_loss.take() {
            losses.push(p.finish(text.lines().count())?);
        }
        Ok(Artifact {
            header: ArtifactHeader {
                schema_version,
                session_lo: session_lo.ok_or(ArtifactError::BadHeader("session_lo"))?,
                session_hi: session_hi.ok_or(ArtifactError::BadHeader("session_hi"))?,
                run_token,
                workload_tag: workload_tag.ok_or(ArtifactError::BadHeader("workload_tag"))?,
                // Required, not defaulted: a missing key is a MALFORMED header, while a present
                // empty one is an honest "nobody declared". Defaulting would erase that.
                content_tag: content_tag.ok_or(ArtifactError::BadHeader("content_tag"))?,
                instrument: instrument.ok_or(ArtifactError::BadHeader("instrument"))?,
                precision_decimals: precision_decimals
                    .ok_or(ArtifactError::BadHeader("precision_decimals"))?,
                regimes: regimes.ok_or(ArtifactError::BadHeader("regimes"))?,
                modes: modes.ok_or(ArtifactError::BadHeader("modes"))?,
                regime_n_distinct: regime_n_distinct
                    .ok_or(ArtifactError::BadHeader("regime_n_distinct"))?,
                // Required, not defaulted, for `content_tag`'s reason: an absent key means an older
                // writer, and defaulting it to `fifo` would invent the one value that makes a
                // refresh-bounded wall clock look explicable.
                present_mode: present_mode.ok_or(ArtifactError::BadHeader("present_mode"))?,
                // The FLAG is required for `content_tag`'s reason -- an absent key would default to
                // "no shim", the one value that makes a diagnostic-mode artifact look like a clean
                // one. The three COUNTS default to zero, because a `false` flag already says they
                // mean nothing and requiring them would refuse a writer that had nothing to say.
                alloc_shim: alloc_shim.ok_or(ArtifactError::BadHeader("alloc_shim"))?,
                alloc_allocs: alloc_allocs.unwrap_or(0),
                alloc_deallocs: alloc_deallocs.unwrap_or(0),
                alloc_bytes: alloc_bytes.unwrap_or(0),
            },
            zones,
            census,
            losses,
        })
    }
}

/// A zone row under construction.
#[derive(Default)]
struct PartialZone {
    zone: Option<u16>,
    label: Option<ZoneLabel>,
    n: Option<u32>,
    median: Option<f64>,
    mean: Option<f64>,
    p95: Option<f64>,
    stddev: Option<f64>,
    begin: Option<f64>,
    end: Option<f64>,
}

/// A `[[loss]]` block while its keys are still arriving.
#[derive(Default)]
struct PartialLoss {
    class: Option<String>,
    count: Option<u64>,
    bytes: Option<u64>,
}

impl PartialLoss {
    /// Every field required, for [`PartialZone::finish`]'s reason: a defaulted `count = 0` is a
    /// measurement of zero drops, which is the confusion this file exists to prevent.
    fn finish(self, line: usize) -> Result<LossRow, ArtifactError> {
        let bad = |why| ArtifactError::Malformed { line, why };
        Ok(LossRow {
            class: self.class.ok_or_else(|| bad("loss row has no `class`"))?,
            count: self.count.ok_or_else(|| bad("loss row has no `count`"))?,
            bytes: self.bytes.ok_or_else(|| bad("loss row has no `bytes`"))?,
        })
    }
}

impl PartialZone {
    /// Every field is required: a row missing one is malformed, not defaulted. A defaulted
    /// `median_ns = 0.0` is a measurement of zero, which is the confusion this campaign keeps
    /// finding.
    fn finish(self, line: usize) -> Result<ZoneRow, ArtifactError> {
        let bad = |why| ArtifactError::Malformed { line, why };
        Ok(ZoneRow {
            zone: self.zone.ok_or_else(|| bad("zone row has no `id`"))?,
            label: self.label.ok_or_else(|| bad("zone row has no `label`"))?,
            n: self.n.ok_or_else(|| bad("zone row has no `n`"))?,
            median_ns: self.median.ok_or_else(|| bad("zone row has no `median_ns`"))?,
            mean_ns: self.mean.ok_or_else(|| bad("zone row has no `mean_ns`"))?,
            p95_ns: self.p95.ok_or_else(|| bad("zone row has no `p95_ns`"))?,
            stddev_ns: self.stddev.ok_or_else(|| bad("zone row has no `stddev_ns`"))?,
            begin_off_ns: self.begin.ok_or_else(|| bad("zone row has no `begin_off_ns`"))?,
            end_off_ns: self.end.ok_or_else(|| bad("zone row has no `end_off_ns`"))?,
        })
    }
}

/// Splits `key = value`, or `None` when the line is not one.
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Strips one layer of double quotes.
fn unquote(v: &str) -> &str {
    v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v)
}
