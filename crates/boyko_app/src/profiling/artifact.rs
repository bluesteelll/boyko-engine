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
//! # Decision 5 — `WorkloadTag` IS a field
//!
//! `resolve` refuses a `Floor` whose workload tag does not match, and rung 7b builds
//! `docs/PROFILING-FLOOR.md` out of per-session artifacts. A session file that did not carry its own
//! tag would force the aggregator to infer it, and an inference is exactly what the tag exists to
//! prevent.
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
pub const ARTIFACT_SCHEMA_VERSION: u32 = 1;

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
    /// What was measured, so a `Floor` cannot be applied across workloads.
    pub workload_tag: String,
    /// Whether the timestamp instrument was alive.
    pub instrument: Instrument,
    /// [`PRECISION_DECIMALS`] at write time, stated so a reader never has to assume it.
    pub precision_decimals: u8,
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

/// A whole artifact: header, per-zone rows, label census.
#[derive(Clone, PartialEq, Debug)]
pub struct Artifact {
    /// The header, checked before any row is parsed.
    pub header: ArtifactHeader,
    /// One row per zone.
    pub zones: Vec<ZoneRow>,
    /// The label census for the same window.
    pub census: LabelCensus,
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
        let _ = writeln!(s, "instrument = \"{}\"", h.instrument.as_str());
        let _ = writeln!(s, "precision_decimals = {}", h.precision_decimals);
        let c = &self.census;
        let _ = writeln!(s, "census_measured = {}", c.measured);
        let _ = writeln!(s, "census_not_bracketed = {}", c.not_bracketed);
        let _ = writeln!(s, "census_lost = {}", c.lost);
        let _ = writeln!(s, "census_torn = {}", c.torn);
        for z in &self.zones {
            let _ = writeln!(s, "\n[[zone]]");
            let _ = writeln!(s, "id = {}", z.zone);
            let _ = writeln!(s, "label = \"{}\"", z.label.as_str());
            let _ = writeln!(s, "n = {}", z.n);
            let _ = writeln!(s, "median_ns = {}", ns(z.median_ns));
            let _ = writeln!(s, "mean_ns = {}", ns(z.mean_ns));
            let _ = writeln!(s, "p95_ns = {}", ns(z.p95_ns));
            let _ = writeln!(s, "begin_off_ns = {}", ns(z.begin_off_ns));
            let _ = writeln!(s, "end_off_ns = {}", ns(z.end_off_ns));
        }
        s
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
        let mut instrument = None;
        let mut precision_decimals = None;
        let mut census = LabelCensus::default();
        let mut zones: Vec<ZoneRow> = Vec::new();
        let mut cur: Option<PartialZone> = None;

        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[zone]]" {
                if let Some(p) = cur.take() {
                    zones.push(p.finish(i + 1)?);
                }
                cur = Some(PartialZone::default());
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

        Ok(Artifact {
            header: ArtifactHeader {
                schema_version,
                session_lo: session_lo.ok_or(ArtifactError::BadHeader("session_lo"))?,
                session_hi: session_hi.ok_or(ArtifactError::BadHeader("session_hi"))?,
                run_token,
                workload_tag: workload_tag.ok_or(ArtifactError::BadHeader("workload_tag"))?,
                instrument: instrument.ok_or(ArtifactError::BadHeader("instrument"))?,
                precision_decimals: precision_decimals
                    .ok_or(ArtifactError::BadHeader("precision_decimals"))?,
            },
            zones,
            census,
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
    begin: Option<f64>,
    end: Option<f64>,
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
