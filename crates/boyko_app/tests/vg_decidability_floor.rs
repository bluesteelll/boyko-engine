//! **VG — the decidability floor.** The smallest relative GPU-timing delta this box can resolve.
//!
//! # Why this rung exists, and why it is not optional
//!
//! Every rung of the virtual-geometry ladder after R0 is gated on a **measured Δ**
//! (`docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md` §4): R2 is *"measured Δ on R0 corpus, **decidable by
//! R0's floor**"*, R3 is a measured hit rate and marginal yield, R4 a measured curve improvement, R7
//! *"decided by our numbers, not literature"*. **R0 produces no such floor.** The decidability
//! apparatus was removed from `docs/VG-CAMPAIGN-THRESHOLDS.toml` at Rev 8 and parked in the plan's
//! §14; the plan states in its own words that *"R0 builds no harness and measures no delta, so there
//! is nothing here for K3 to be true or false about."* So R2's gate cites a floor that does not
//! exist, and the whole ladder below it inherits that.
//!
//! K3 — *"Undecidable harness. If the resolvable delta exceeds the delta we intend to claim, no
//! result from this campaign is defensible"* — is the kill this rung tests. It is not hypothetical
//! here: the research document records that this project *"has already recorded a bench that does
//! not reproduce above N=128 with ~21% spread, and shipped a '22×' result measured inside that
//! regime."*
//!
//! # The design: a NULL experiment
//!
//! The floor is measured by running the **same** bench, on the **same** scene, in the **same**
//! configuration, across N separate processes. Nothing differs between sessions, so every difference
//! observed is instrument plus environment. **A delta smaller than that spread is not resolvable by
//! construction** — no statistical treatment recovers a signal below the noise of the thing
//! measuring it.
//!
//! This deliberately measures the SHIPPED bench class the later rungs will use
//! (`vb_p1d_cull_shade_bench`'s VB froxel cull/shade GPU timestamps) rather than a synthetic
//! stand-in, because a floor established on a different instrument bounds nothing about this one.
//!
//! # Profiling rung 7 — MIGRATED to the artifact, and that changes three things
//!
//! This file used to parse the shipped bench's own stdout (`VB-P1d …`). Rung 7 deletes that channel,
//! so the sessions now run the **zone recorder** (`BOYKO_VB_ZONE`) and read
//! `boyko_app::profiling::artifact` files the parent names, stamps and deletes. By this rung's own
//! rule — *"a floor established on a different instrument bounds nothing about this one"* — **every
//! number in `docs/VG-DECIDABILITY-FLOOR.md` published before this migration is invalidated**, which
//! is exactly why rung 7b re-measures rather than re-uses.
//!
//! **1. `froxel_total_ns` is GONE, and dropping it was decided by measurement.** It was a per-frame
//! SUM of three brackets, averaged; the artifact reduces each zone independently, and composing
//! after reduction is the mistake `VbBenchTables::end_off_ns` already records ("a harness cannot get
//! one by adding the other two after they are reduced"). The alternative was a composite zone in the
//! reducer, built solely to reproduce it. MEASURED against the committed floor's own table: its CV
//! was **2.9 %**, below `froxel_shade_ns` (3.0 %) in its own leg and well below the worst,
//! `flat_shade_ns` (3.4 %), which is what the floor is actually built from. It has never been the
//! binding statistic, and structurally it will not be — a sum of three partly-independent noise
//! sources lands *between* their relative spreads, never above. So the mechanism was not built.
//!
//! **2. The legs are told apart by the PARENT, not by a missing key.** The old parser distinguished
//! flat from froxel by the absence of the `froxel_*` keys. The recorder brackets `CullReset` and
//! `CullDispatch` unconditionally, so both legs now write zones 0/1/2 and the flat leg simply reads
//! near-zero on the first two. The parent knows which child it spawned; that is a stronger witness
//! than an absence, and it is checked — rung 7c's `config_tag` covers `froxel_light_cull`, so the
//! two legs' artifacts carry **different `workload_tag`s**, and this file asserts they differ. A
//! `BOYKO_VB_FROXEL_FORCE_OFF` that silently did nothing was invisible to the printed channel and is
//! a red here.
//!
//! **3. The statistic is the MEDIAN, where the printed channel published means.** VB-P1d predates
//! the reducer; the artifact's headline is the median, and the deltas later rungs will compare
//! against this floor are medians. A floor over means would bound a different quantity than the one
//! being bounded. Stated rather than inherited, because either is defensible and only one can be
//! used by both sides.
//!
//! # Two statistics, and which one the floor is built from
//!
//! * **Peak-to-peak** `(max − min) / median` — the definition `sv0_deferred_term_bench` already uses
//!   for its own cross-session gate, so this number is comparable to the one existing gate in the
//!   tree. It **grows with session count** (more sessions, more chances for an extreme), so it is
//!   only meaningful beside its `n`.
//! * **Coefficient of variation** `σ / mean` — stable in `n`.
//!
//! ⚠️ **The floor is CV-derived, and that overturned this file's own first design.** The draft
//! adopted the worst peak-to-peak, on the argument that a floor which under-states noise is the one
//! direction that silently blesses wrong constants. The [`DEFAULT_REPEATS`] repetitions refuted it
//! by measurement: repetition floors swing by a large factor between identical runs while the CV
//! barely moves. **A bound that cannot reproduce itself is not a bound, however conservative it
//! looks on any single run.** Peak-to-peak is still reported, because the one existing gate in the
//! tree is written in it; it is not what a new gate should use. The floor is
//! [`FLOOR_SIGMA`] × the worst statistic's CV.
//!
//! Windowed-test conventions: `#[ignore]`, `BOYKO_DISABLE_VALIDATION=1`, one process per session.

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use boyko_app::profiling::artifact::{Artifact, Instrument, ZoneLabel};

/// Sessions per configuration within ONE repetition. More than `[census].cross_run_sessions = 3` on
/// purpose: three samples estimate a spread very poorly, and this rung's output is a BOUND that
/// later gates rest on, so under-estimating it is the dangerous direction.
const DEFAULT_SESSIONS: usize = 7;

/// Independent REPETITIONS of the whole `DEFAULT_SESSIONS`-session experiment.
///
/// ⚠️ **This exists because the first draft measured the floor twice and got 6.3 % and 14.3 %** —
/// the same protocol, the same scene, the same box, a factor of 2.3 apart. So the floor ESTIMATOR
/// is itself noisy, and a single run's peak-to-peak quoted as "the floor" would be exactly the
/// over-confidence this rung exists to prevent, one level up. Repetitions put the estimator's own
/// stability on the page instead of assuming it.
const DEFAULT_REPEATS: usize = 3;

/// Sigmas of separation the floor demands on a SINGLE-session reading. Three: the conventional
/// "clearly not noise" threshold, applied without any `sqrt(n)` credit for repeated sessions, so a
/// rung that runs several sessions per condition can only do better than this bound.
const FLOOR_SIGMA: f64 = 3.0;

/// Where the floor is written.
const FLOOR_DOC: &str = "../../docs/VG-DECIDABILITY-FLOOR.md";

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

// ===============================================================================================
// The statistic
// ===============================================================================================

/// One configuration's cross-session summary. All ratios are relative (dimensionless).
#[derive(Debug, Clone)]
struct Spread {
    label: String,
    samples: Vec<f64>,
    median: f64,
    mean: f64,
    /// `(max − min) / median` — comparable to `sv0_deferred_term_bench`'s gate, grows with `n`.
    peak_to_peak: f64,
    /// `σ / mean`, sample standard deviation. Stable in `n`.
    cv: f64,
}

/// Summarises one configuration's session samples.
///
/// # Panics
///
/// Panics on fewer than two samples: a spread over one sample is zero, which would report a
/// PERFECT instrument from a single run — the exact false-green this rung exists to prevent.
fn summarise(label: &str, samples: &[f64]) -> Spread {
    assert!(
        samples.len() >= 2,
        "the floor needs at least two sessions; one sample reports a spread of zero, which would \
         certify a perfect instrument from a single run"
    );
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("bench samples are finite"));
    let n = sorted.len();
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) * 0.5
    } else {
        sorted[n / 2]
    };
    let mean = sorted.iter().sum::<f64>() / n as f64;
    // Sample (n-1) standard deviation: the samples ARE a sample of the process, not its population.
    let var = sorted.iter().map(|s| (s - mean) * (s - mean)).sum::<f64>() / (n as f64 - 1.0);
    Spread {
        label: label.to_string(),
        samples: sorted.clone(),
        median,
        mean,
        peak_to_peak: if median > 0.0 { (sorted[n - 1] - sorted[0]) / median } else { f64::INFINITY },
        cv: if mean > 0.0 { var.sqrt() / mean } else { f64::INFINITY },
    }
}

// ===============================================================================================
// Reading the session's artifact
// ===============================================================================================

/// The zone ids this rung reads, and the name each one is published under.
///
/// `ZONE_BASE_VB` is `0`, so a VB pass's slot IS its zone id. Named here rather than imported
/// because rung 7 step 5 DID delete `VbTimedPass` with its collector, and a floor instrument that stops
/// compiling when an enum is retired is a floor instrument nobody can re-run.
const ZONE_CULL_RESET: u16 = 0;
const ZONE_CULL_DISPATCH: u16 = 1;
const ZONE_SHADE: u16 = 2;

/// One statistic pulled out of a session's artifact: a zone id and the label it is published under.
struct Reading {
    zone: u16,
    label: &'static str,
}

/// The froxel leg's three, in report order — the whole-pass sum the printed channel published is
/// deliberately absent (module doc, point 1).
const FROXEL_READINGS: [Reading; 3] = [
    Reading { zone: ZONE_CULL_RESET, label: "cull_reset_ns" },
    Reading { zone: ZONE_CULL_DISPATCH, label: "cull_dispatch_ns" },
    Reading { zone: ZONE_SHADE, label: "froxel_shade_ns" },
];

/// The flat leg's one. The SAME zone as the froxel leg's shade — the legs differ by configuration,
/// not by which bracket runs, which is why the parent has to know which child it spawned.
const FLAT_READINGS: [Reading; 1] = [Reading { zone: ZONE_SHADE, label: "flat_shade_ns" }];

/// The MEDIAN of `zone`'s window, or `None` when the run did not measure it.
///
/// `None` on a missing row, on a non-`Measured` label, and on a dead instrument — three different
/// reasons a session has no number, all of which must keep it OUT of the pool rather than
/// contribute a zero. A floor pooled from zeros understates the spread, which is the one direction
/// this rung exists to prevent.
fn zone_median(art: &Artifact, zone: u16) -> Option<f64> {
    if art.header.instrument != Instrument::Live {
        return None;
    }
    art.zones
        .iter()
        .find(|z| z.zone == zone)
        .filter(|z| z.label == ZoneLabel::Measured)
        .map(|z| z.median_ns)
}

// ===============================================================================================
// Driving the shipped bench binary
// ===============================================================================================

/// Locates the sibling `vb_p1d_cull_shade_bench` test binary beside this one.
///
/// Shelling out to `cargo` from inside a `cargo test` would contend on the build lock; the harness
/// binaries are all emitted into the same directory with a hash suffix, so the newest match is the
/// one this run was built against.
fn bench_binary() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("vb_p1d_cull_shade_bench-") && name.ends_with(".exe") {
            let t = entry.metadata().ok()?.modified().ok()?;
            if best.as_ref().is_none_or(|(bt, _)| t > *bt) {
                best = Some((t, entry.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// The light rig every session runs, declared ONCE and used twice — to configure the child and to
/// name the workload in its artifact. A floor is a property of its content as much as of the box
/// (rung 7c), and two spellings of "fourteen lights" is how the tag comes to describe another run.
const N_PS: &str = "14";
/// `BOYKO_VB_BENCH_RIG`'s default. Named for the same one-source-of-truth reason.
const RIG: &str = "kronecker";

/// Runs ONE session and returns the artifact it wrote, or `None` when it wrote none.
///
/// The parent chooses the path AND the run token, then deletes the file first: with 42 sequential
/// children a fixed path is a stale-read generator, and a token the child merely echoes is the only
/// field that can catch staleness *within* one run (`artifact.rs`'s Decision 4).
fn run_session(exe: &PathBuf, froxel: bool, tag: &str) -> Option<Artifact> {
    let mut path = std::env::temp_dir();
    path.push(format!("boyko_vg_floor_{tag}.toml"));
    let _ = std::fs::remove_file(&path);
    let token = format!("vg-floor-{tag}");

    let mut cmd = Command::new(exe);
    cmd.args(["--ignored", "--test-threads=1", "--nocapture"])
        .env("BOYKO_DISABLE_VALIDATION", "1")
        // The ZONE recorder, not the retired collector. `GpuSceneBundles::boot` refuses the two
        // together, so the removal below is not defensive: an operator with the old knob exported
        // would otherwise fail every session at boot.
        .env("BOYKO_VB_ZONE", "1")
        .env_remove("BOYKO_VB_BENCH")
        .env("BOYKO_PROFILE_ARTIFACT", &path)
        .env("BOYKO_PROFILE_RUN_TOKEN", &token)
        // Rung 7c: what the engine cannot derive. Without it the artifact refuses to be a floor
        // source at all, which is the whole point of that rung's strict rule.
        .env("BOYKO_PROFILE_WORKLOAD", format!("n{N_PS}_{RIG}"))
        .env("BOYKO_VB_BENCH_LIGHTS", N_PS)
        .env("BOYKO_VB_BENCH_RIG", RIG)
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS");
    if froxel {
        cmd.env_remove("BOYKO_VB_FROXEL_FORCE_OFF");
    } else {
        cmd.env("BOYKO_VB_FROXEL_FORCE_OFF", "1");
    }
    let out = cmd.output().expect("the bench binary runs");
    if !path.is_file() {
        eprintln!(
            "VG-floor: session {tag} wrote no artifact at {}. stdout tail:\n{}",
            path.display(),
            String::from_utf8_lossy(&out.stdout).lines().rev().take(6).collect::<Vec<_>>().join("\n")
        );
        return None;
    }
    // Read with the token the parent chose: a leftover from an earlier child is refused on the
    // header rather than pooled as this session's reading.
    let art = match Artifact::read(&path, &token) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("VG-floor: session {tag}'s artifact is unusable: {e}");
            return None;
        }
    };
    // ...and it must be usable AS A FLOOR, which is a stronger statement than "it parsed".
    if let Err(e) = art.floor_source() {
        eprintln!("VG-floor: session {tag}'s artifact cannot serve as a floor: {e}");
        return None;
    }
    let _ = std::fs::remove_file(&path);
    Some(art)
}

// ===============================================================================================
// The measurement
// ===============================================================================================

/// **Measures the decidability floor and writes `docs/VG-DECIDABILITY-FLOOR.md`.**
///
/// SKIPS BY NAME when the sibling bench binary has not been built — a rung that silently measures
/// nothing is indistinguishable from one that measured a perfect instrument.
#[test]
#[ignore = "needs a real windowed GPU device; runs N separate bench processes per configuration"]
fn vg_decidability_floor_measure() {
    let Some(exe) = bench_binary() else {
        eprintln!(
            "SKIP vg_decidability_floor_measure: the sibling `vb_p1d_cull_shade_bench` binary is \
             not built. Run `cargo test -p boyko-app --test vb_p1d_cull_shade_bench --no-run` \
             first. NOTHING about the decidability floor is measured by this run."
        );
        return;
    };
    let sessions: usize = std::env::var("BOYKO_VG_FLOOR_SESSIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SESSIONS);
    let repeats: usize = std::env::var("BOYKO_VG_FLOOR_REPEATS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_REPEATS);

    // Statistics of very different magnitudes on purpose: the fixed-cost brackets are expected to
    // be the tightest and the whole-shade one the loosest, and a floor quoted from only one of them
    // would be a floor for one statistic quoted as if it were the instrument's. The printed
    // channel's fourth froxel figure (`froxel_total_ns`) is deliberately absent — see the module
    // doc's point 1 for the measurement that says dropping it costs nothing.

    // Per REPETITION: an independent run of the whole session set. Repetition floors are what say
    // whether one floor number can be trusted at all.
    let mut per_repeat: Vec<Vec<Spread>> = Vec::with_capacity(repeats);
    // Pooled across every repetition: the estimate that actually gets more sessions behind it.
    let mut pooled_froxel: Vec<Vec<f64>> = vec![Vec::new(); FROXEL_READINGS.len()];
    let mut pooled_flat: Vec<Vec<f64>> = vec![Vec::new(); FLAT_READINGS.len()];
    // The two legs' DERIVED workload tags, collected to be compared. Rung 7c's `config_tag` covers
    // `froxel_light_cull`, so a `BOYKO_VB_FROXEL_FORCE_OFF` that silently did nothing shows up as
    // ONE tag on both legs — a state the printed channel could not observe at all.
    let mut froxel_tag: Option<String> = None;
    let mut flat_tag: Option<String> = None;

    for r in 0..repeats {
        let mut froxel: Vec<Vec<f64>> = vec![Vec::new(); FROXEL_READINGS.len()];
        let mut flat: Vec<Vec<f64>> = vec![Vec::new(); FLAT_READINGS.len()];
        for k in 0..sessions {
            if let Some(art) = run_session(&exe, true, &format!("r{r}s{k}froxel")) {
                froxel_tag.get_or_insert_with(|| art.header.workload_tag.clone());
                for (i, rd) in FROXEL_READINGS.iter().enumerate() {
                    if let Some(v) = zone_median(&art, rd.zone) {
                        froxel[i].push(v);
                    }
                }
            }
            if let Some(art) = run_session(&exe, false, &format!("r{r}s{k}flat")) {
                flat_tag.get_or_insert_with(|| art.header.workload_tag.clone());
                for (i, rd) in FLAT_READINGS.iter().enumerate() {
                    if let Some(v) = zone_median(&art, rd.zone) {
                        flat[i].push(v);
                    }
                }
            }
        }
        let mut rep: Vec<Spread> = Vec::new();
        for (i, rd) in FROXEL_READINGS.iter().enumerate() {
            assert_eq!(
                froxel[i].len(),
                sessions,
                "repetition {r}: the froxel leg measured `{}` on {} of {sessions} sessions -- a \
                 session whose artifact was missing, unreadable or not `Measured` measured \
                 NOTHING, and pooling only the survivors would UNDERSTATE the spread",
                rd.label,
                froxel[i].len()
            );
            pooled_froxel[i].extend_from_slice(&froxel[i]);
            rep.push(summarise(rd.label, &froxel[i]));
        }
        for (i, rd) in FLAT_READINGS.iter().enumerate() {
            assert_eq!(
                flat[i].len(),
                sessions,
                "repetition {r}: the flat leg measured `{}` on {} of {sessions} sessions",
                rd.label,
                flat[i].len()
            );
            pooled_flat[i].extend_from_slice(&flat[i]);
            rep.push(summarise(rd.label, &flat[i]));
        }
        let rf = FLOOR_SIGMA * rep.iter().map(|x| x.cv).fold(0.0_f64, f64::max);
        eprintln!("VG-floor: repetition {} of {repeats} done -- its floor = {:.1}%", r + 1, 100.0 * rf);
        per_repeat.push(rep);
    }

    // THE TWO LEGS MUST BE TWO WORKLOADS, not one condition measured twice. This is the clause the
    // stdout channel had no way to state: it told the legs apart by WHICH KEYS WERE PRINTED, so a
    // force-off knob that did nothing produced a froxel line from the "flat" session and the
    // absence-based parser would have read it as the flat leg reporting nothing.
    let (ft, lt) = (
        froxel_tag.expect("invariant: the froxel leg produced at least one artifact"),
        flat_tag.expect("invariant: the flat leg produced at least one artifact"),
    );
    assert_ne!(
        ft, lt,
        "both legs reported the SAME derived workload tag ({ft}), so BOYKO_VB_FROXEL_FORCE_OFF did \
         not change the boot-resolved configuration -- this rung measured ONE condition twice and \
         would have published it as a null experiment across two"
    );

    let mut spreads: Vec<Spread> = Vec::new();
    for (i, rd) in FROXEL_READINGS.iter().enumerate() {
        spreads.push(summarise(rd.label, &pooled_froxel[i]));
    }
    for (i, rd) in FLAT_READINGS.iter().enumerate() {
        spreads.push(summarise(rd.label, &pooled_flat[i]));
    }

    // THE FLOOR is CV-DERIVED, and that is a correction this rung MEASURED against its own first
    // design. The draft adopted the worst peak-to-peak, for comparability with
    // `sv0_deferred_term_bench`'s gate and because peak-to-peak is the conservative reading. The
    // repetitions refuted it: repetition floors swung by a factor of ~4 while the CV moved little,
    // so peak-to-peak is not a floor at all -- it is a statistic that cannot reproduce itself, and
    // a bound that changes 4x between identical runs bounds nothing.
    //
    // `FLOOR_SIGMA * CV` of the WORST statistic. Three sigma on a SINGLE-session reading, taking no
    // sqrt(n) credit for repeated sessions -- a later rung that runs n sessions per condition may
    // scale this down by sqrt(n), which is exactly why the CV rather than the derived figure is the
    // number to carry forward.
    let worst = spreads
        .iter()
        .max_by(|a, b| a.cv.partial_cmp(&b.cv).expect("finite"))
        .expect("at least one statistic");
    let floor = FLOOR_SIGMA * worst.cv;

    write_floor_doc(&spreads, floor, worst, sessions, repeats, &per_repeat, &ft, &lt);

    for s in &spreads {
        eprintln!(
            "VG-floor {}: median={:.1} ns  peak-to-peak={:.1}%  CV={:.1}%  n={}",
            s.label,
            s.median,
            100.0 * s.peak_to_peak,
            100.0 * s.cv,
            s.samples.len()
        );
    }
    eprintln!("VG-floor: FLOOR = {:.1}% (worst statistic: {})", 100.0 * floor, worst.label);

    // The one assertion this rung makes about itself: a floor of zero would mean every session
    // returned an identical number, which for a GPU wall-clock across separate processes means the
    // measurement did not happen.
    assert!(
        floor > 0.0,
        "a decidability floor of exactly zero across {sessions} processes is not a perfect \
         instrument, it is evidence the bench did not vary because it did not run"
    );
}

/// Writes the floor and the evidence behind it.
#[allow(clippy::too_many_arguments)]
fn write_floor_doc(
    spreads: &[Spread],
    floor: f64,
    worst: &Spread,
    sessions: usize,
    repeats: usize,
    per_repeat: &[Vec<Spread>],
    froxel_tag: &str,
    flat_tag: &str,
) {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(4096);
    out.push_str("# VG — the decidability floor — MACHINE-WRITTEN by `vg_decidability_floor_measure`\n\n");
    // ONE emission PER OUTPUT LINE. `\`-continued literals carry the source's indentation into the
    // file and four spaces is a markdown code block; that defect shipped once in this function and
    // was re-introduced in the very edit that fixed it elsewhere, which is what "avoidable" buys
    // you. This shape has nowhere for the indentation to come from.
    let _ = writeln!(
        out,
        "**This run measured a floor of {:.1} %** — three sigma on a single-session reading, from a worst-statistic CV of {:.1} % (`{}`).\n",
        100.0 * floor,
        100.0 * worst.cv,
        worst.label
    );
    // ⚠️ NO SERIES LITERAL HERE. An earlier draft hardcoded "the first two protocols returned
    // 6.5 % and 17.7 %", and a third sitting made that sentence false the day it was written — a
    // self-rotting number inside a MACHINE-WRITTEN file, which is the worst place for one. The
    // cross-sitting series lives in `docs/diagnostics/profiling/05-LADDER-GATES.md`, where a human
    // appends to it; this file states the rule and this run's own repetition span, both of which it
    // can compute.
    out.push_str("⚠️ **Do not read that as \"the floor\".** The estimator moves — by a factor of several between IDENTICAL protocols on this box, measured on both the retired stdout channel and the artifact channel that replaced it. The cross-sitting series is kept in `docs/diagnostics/profiling/05-LADDER-GATES.md` (profiling rung 7b); this run's own repetition span is tabulated below. **The migration did not make the instrument quieter.** The defensible output of this rung is the RULE in the next section, not the number in this one.\n\n");
    out.push_str(
        "Measured as a **NULL EXPERIMENT**: the shipped `vb_p1d_cull_shade_bench` class, same scene, \
         same configuration, run in separate processes. Nothing differs between sessions, so every \
         difference below is instrument plus environment. **A delta smaller than this is not \
         resolvable by construction** — no statistical treatment recovers a signal from beneath the \
         noise of the thing measuring it.\n\n",
    );
    // ⚠️ THE CHANNEL AND THE WORKLOAD, IN THE DOCUMENT. A floor bounds the instrument AND the
    // workload it was taken on; a reader who cannot tell which of either produced these numbers
    // cannot tell whether the floor applies to what they are about to claim. Profiling rung 7
    // replaced the instrument (stdout -> artifact) and rung 7c made the workload nameable, so both
    // are stamped here rather than left to the reader to infer from a filename.
    //
    // Emitted ONE `writeln!` PER LINE, with no `\`-continuations. Markdown treats a four-space
    // indent as a code block, and a continued Rust string literal carries the source's own
    // indentation into the file — measured: the first draft of this block rendered as four fenced
    // paragraphs. A doc generator that cannot be read is a doc generator that will be ignored.
    out.push_str("## The channel and the workload these numbers belong to\n\n");
    out.push_str(
        "Read through the **profiling artifact** (`BOYKO_VB_ZONE` + \
         `boyko_app::profiling::artifact`), NOT the retired `VB-P1d` stdout line — profiling \
         rung 7. The statistic is each zone's **median**, where the printed channel published \
         means.\n\n",
    );
    out.push_str("| leg | derived `workload_tag` | declared `content_tag` |\n|---|---|---|\n");
    let _ = writeln!(out, "| froxel | `{froxel_tag}` | `n{N_PS}_{RIG}` |");
    let _ = writeln!(out, "| flat | `{flat_tag}` | `n{N_PS}_{RIG}` |");
    out.push_str(
        "\n⚠️ **The two tags differ, and that is asserted rather than assumed.** They are derived \
         from the whole boot-resolved render path, so a `BOYKO_VB_FROXEL_FORCE_OFF` that changed \
         nothing would give one tag on both rows and fail the run — a null experiment across two \
         conditions that were secretly one. **Any floor published before rung 7 was taken on a \
         different instrument and bounds nothing about this one**, by this rung's own rule.\n\n",
    );
    let _ = writeln!(
        out,
        "Protocol: **{repeats} independent repetitions × {sessions} sessions** per configuration \
         ({} bench processes in total).\n",
        repeats * sessions * 2
    );

    // ⚠️ The estimator's own instability goes FIRST, because it is the finding, and because it
    // bounds how much the headline above is worth.
    out.push_str(
        "## ⚠️ THE FLOOR IS NOT A CONSTANT — and that, not any single number, is this rung's result\n\n\
         ⚠️ The four runs below were taken on the **RETIRED stdout channel** (means over `VB-P1d` \
         lines), before profiling rung 7 moved this rung to the artifact and to medians. They are \
         kept because the FINDING they establish is about the estimator and the box, not about the \
         channel — but their numbers are not comparable with the table further down, and nothing \
         here claims the new channel is quieter: that would need this same repeated protocol run \
         on both, which no sitting has done.\n\n\
         This protocol was run four times while it was being built. The floors it reported, in \
         order, with what changed between them:\n\n\
         | run | protocol | floor | note |\n|---|---|---|---|\n\
         | 1 | 7 sessions, peak-to-peak | **6.3 %** | first measurement |\n\
         | 2 | 7 sessions, peak-to-peak | **14.3 %** | *identical protocol*, 2.3× higher |\n\
         | 3 | 3 × 7, CV-derived | **4.7 %** | statistic changed after run 2 refuted peak-to-peak |\n\
         | 4 | 3 × 7, CV-derived | **13.5 %** | *identical protocol*, 2.9× higher |\n\n\
         Runs 1↔2 and 3↔4 are pairs of **identical** protocols on the same box and the same scene. \
         They differ by roughly **3×**. Changing the statistic (peak-to-peak → CV) did not fix it, \
         and neither did tripling the sessions.\n\n\
         **So the operational result is not a threshold, it is a rule:**\n\n\
         > **On this box, a claimed GPU-timing delta below ~15 % is not defensible without a NULL \
         CONTROL measured in the same sitting.** The floor drifts on a timescale shorter than the \
         gap between two of these runs — thermal state, driver residency, background load — so a \
         floor measured yesterday does not bound a delta measured today.\n\n\
         This is a stronger and more useful finding than a constant would have been, and it fully \
         explains the failure the research document records: a *\"22× result measured inside\"* a \
         regime that *\"does not reproduce\"*. The remedy is not a better number here; it is that \
         every future rung claiming a delta runs its own A/A control beside its A/B.\n\n\
         The single run that produced the table below repeats the whole experiment \
         and publishes each repetition's own floor, so the drift is visible within one sitting too:\n\n",
    );
    out.push_str("| repetition | floor (worst peak-to-peak) |\n|---|---|\n");
    for (i, rep) in per_repeat.iter().enumerate() {
        let rf = FLOOR_SIGMA * rep.iter().map(|x| x.cv).fold(0.0_f64, f64::max);
        let _ = writeln!(out, "| {} | {:.1} % |", i + 1, 100.0 * rf);
    }
    let reps: Vec<f64> = per_repeat
        .iter()
        .map(|r| FLOOR_SIGMA * r.iter().map(|x| x.cv).fold(0.0_f64, f64::max))
        .collect();
    if reps.len() >= 2 {
        let lo = reps.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = reps.iter().cloned().fold(0.0_f64, f64::max);
        let _ = writeln!(
            out,
            "\n**Repetition floors span {:.1} %–{:.1} %, a factor of {:.2}.** Read the headline as \
             an order of magnitude, never as a constant. The table below pools every session, which \
             is the estimate with the most evidence behind it.\n",
            100.0 * lo,
            100.0 * hi,
            if lo > 0.0 { hi / lo } else { f64::INFINITY }
        );
    }

    out.push_str(
        "| statistic | median (ns) | mean (ns) | peak-to-peak | CV | samples |\n\
         |---|---|---|---|---|---|\n",
    );
    for s in spreads {
        let _ = writeln!(
            out,
            "| `{}` | {:.1} | {:.1} | **{:.1} %** | {:.1} % | {:?} |",
            s.label,
            s.median,
            s.mean,
            100.0 * s.peak_to_peak,
            100.0 * s.cv,
            s.samples.iter().map(|v| *v as u64).collect::<Vec<_>>()
        );
    }

    let _ = writeln!(
        out,
        "\n**The floor is {:.0} sigma × the WORST statistic's CV — `{}` at {:.1} %, giving \
         {:.1} %** — worst rather than best or average, because a campaign quoting its tightest \
         statistic as \"the floor\" would be certifying deltas it cannot resolve on any other one.\n",
        FLOOR_SIGMA,
        worst.label,
        100.0 * worst.cv,
        100.0 * floor
    );

    out.push_str(
        "## What this decides\n\n\
         **K3 — the undecidable harness** — is the kill this measures. Any rung claiming a delta \
         **below** the figure above is not defensible on this box, whatever the arithmetic around \
         it. The research ladder's R2 is the immediate case: its own expected magnitude on this \
         content is stated as *\"near zero\"*, so its gate — *\"measured Δ, decidable by R0's \
         floor\"* — is unsatisfiable in **both** directions at once. R2 still has value, but that \
         value is de-risking the cull-pass declaration, compaction, indirect barriers and count \
         buffers; it is not the delta, and its gate should say so.\n\n\
         ## Two statistics, and why both are here\n\n\
         **Peak-to-peak** `(max − min) / median` is the definition `sv0_deferred_term_bench` already \
         uses for its own cross-session gate, so these numbers are comparable to the one existing \
         gate in the tree. ⚠️ It **grows with session count**, so it is only meaningful beside its \
         `n` — which is why `n` is printed above. That growth is in the safe direction for a floor.\n\n\
         **CV** `σ / mean` is stable in `n`, and it is what the floor is built from.\n\n\
         ⚠️ **That choice OVERTURNED this rung's own first design, by measurement.** The draft \
         adopted the worst peak-to-peak, on the argument that a floor which under-states noise is \
         the one direction that silently blesses wrong constants — the failure this project has \
         already recorded once. The repetitions refuted it: peak-to-peak floors swung ~4× between \
         identical runs while the CV barely moved. **A bound that cannot reproduce itself is not a \
         bound, however conservative it looks on any single run.** Peak-to-peak stays in the table \
         because the one existing gate in the tree is written in it; it is not what a new gate \
         should use.\n\n\
         ## What this does NOT decide\n\n\
         * **It is one box.** The floor is a property of this GPU, this driver and this machine's \
           background load, not of the engine.\n\
         * **It is one bench class.** GPU timestamp brackets around compute dispatches. A CPU-side \
           or end-to-end frame-time measurement has its own floor and does not inherit this one.\n\
         * ⚠️ **It is one CONFIGURATION, and this bounds what it contradicts.** These sessions ran \
           the bench's default light rig. The research document's *\"does not reproduce above N=128 \
           with ~21% spread\"* is a reading at a much heavier configuration, and nothing here \
           refutes it: a floor is a property of the workload as much as of the box, and a rung that \
           measures at a different scale must re-measure its own floor rather than cite this one. \
           What this figure DOES establish is that the class is not hopeless — the noise is single- \
           digit percent where the workload is light, so a rung with a large enough effect can be \
           decidable here.\n\
         * **It is not a confidence interval.** It bounds what is resolvable; it does not say how \
           many sessions a future rung needs to resolve a given delta. That is the CV's job and it \
           is recorded above rather than applied here.\n\
         * **No clock pinning was applied.** The floor therefore includes driver/OS clock \
           behaviour, which is what a real measurement on this box would also include. A pinned-clock \
           floor would be tighter and would describe a machine nobody measures on.\n",
    );

    let dest = repo_path(FLOOR_DOC);
    std::fs::write(&dest, out).expect("the floor document is writable");
    eprintln!("VG-floor: written -> {}", dest.display());
}

// ===============================================================================================
// Device-free checks of the arithmetic this rung rests on
// ===============================================================================================

#[test]
fn the_spread_statistics_are_what_they_claim() {
    // Peak-to-peak over the median, and the sample (n-1) CV. Hand-checkable values.
    let s = summarise("x", &[90.0, 100.0, 110.0]);
    assert_eq!(s.median, 100.0);
    assert!((s.peak_to_peak - 0.20).abs() < 1e-12, "20/100");
    assert!((s.cv - (10.0 / 100.0)).abs() < 1e-12, "sd 10 over mean 100");
}

#[test]
fn peak_to_peak_grows_with_sessions_which_is_why_n_is_reported() {
    // The property the doc warns about, asserted rather than described: the same underlying process
    // sampled more times yields a WIDER peak-to-peak, so a floor quoted without its `n` is not a
    // floor. Nested samples make this exact rather than probabilistic.
    let few = summarise("few", &[98.0, 100.0, 102.0]);
    let many = summarise("many", &[95.0, 98.0, 100.0, 102.0, 105.0]);
    assert!(
        many.peak_to_peak > few.peak_to_peak,
        "more sessions must not report a TIGHTER instrument: {:.4} vs {:.4}",
        many.peak_to_peak,
        few.peak_to_peak
    );
}

#[test]
fn a_single_session_cannot_certify_an_instrument() {
    // The false-green this rung is built to refuse: one sample has zero spread.
    let r = std::panic::catch_unwind(|| summarise("one", &[100.0]));
    assert!(r.is_err(), "one session must be refused, not summarised as a perfect instrument");
}

/// Builds an artifact the way a session would, so the reader below has something with the real
/// shape rather than a hand-rolled stand-in.
fn artifact_with(zones: &[(u16, ZoneLabel, f64)], instrument: Instrument) -> Artifact {
    use boyko_app::profiling::artifact::{
        ARTIFACT_SCHEMA_VERSION, ArtifactHeader, LabelCensus, PRECISION_DECIMALS, ZoneRow,
    };
    Artifact {
        header: ArtifactHeader {
            schema_version: ARTIFACT_SCHEMA_VERSION,
            session_lo: 1,
            session_hi: 2,
            run_token: "t".into(),
            workload_tag: "visibilitybuffer_mesh#deadbeef".into(),
            content_tag: format!("n{N_PS}_{RIG}"),
            // Decision 7: a floor session is one regime by construction; a worker that saw two
            // is what `vg_occ_split_timing.rs` rejects, and this rung's sessions never force one.
            regimes: "none".into(),
            modes: "off".into(),
            regime_n_distinct: 1,
            instrument,
            precision_decimals: PRECISION_DECIMALS,
        },
        zones: zones
            .iter()
            .map(|(zone, label, median)| ZoneRow {
                zone: *zone,
                label: *label,
                n: 30,
                median_ns: *median,
                mean_ns: *median,
                p95_ns: *median,
                begin_off_ns: 0.0,
                end_off_ns: *median,
            })
            .collect(),
        census: LabelCensus { measured: 30, ..LabelCensus::default() },
    }
}

/// **The reader takes a number only when the run actually measured one**, and the three ways it
/// must refuse are three different things a session can be.
///
/// This replaces the stdout parser the printed channel needed. That parser's own hard-won lesson —
/// libtest writes its progress line without a trailing newline, so `starts_with` found nothing in
/// fourteen consecutive sessions — dies with the channel: an artifact is a file with a header, not
/// a line that shares its row with whatever else was printing.
#[test]
fn a_reading_is_taken_only_from_a_measured_zone() {
    let good = artifact_with(
        &[
            (ZONE_CULL_RESET, ZoneLabel::Measured, 575.0),
            (ZONE_CULL_DISPATCH, ZoneLabel::Measured, 12677.5),
            (ZONE_SHADE, ZoneLabel::Measured, 26349.4),
        ],
        Instrument::Live,
    );
    assert_eq!(zone_median(&good, ZONE_CULL_RESET), Some(575.0));
    assert_eq!(zone_median(&good, ZONE_SHADE), Some(26349.4));

    // 1. A zone the window never bracketed. `NotBracketed` reads ~0 like a genuinely free pass, so
    //    taking its median would pool a zero and UNDERSTATE the spread — the one direction this
    //    rung exists to prevent.
    let unbracketed =
        artifact_with(&[(ZONE_SHADE, ZoneLabel::NotBracketed, 0.0)], Instrument::Live);
    assert_eq!(zone_median(&unbracketed, ZONE_SHADE), None);

    // 2. A zone whose row is absent entirely.
    let missing = artifact_with(&[(ZONE_CULL_RESET, ZoneLabel::Measured, 1.0)], Instrument::Live);
    assert_eq!(zone_median(&missing, ZONE_SHADE), None);

    // 3. A DEAD INSTRUMENT. The rows may carry numbers; they are not measurements, and a floor
    //    pooled from them would be a floor for a device that declined to time anything.
    let dead =
        artifact_with(&[(ZONE_SHADE, ZoneLabel::Measured, 26349.4)], Instrument::NoTimestamps);
    assert_eq!(zone_median(&dead, ZONE_SHADE), None);
}

/// The two legs read the SAME zone, which is why the parent has to know which child it spawned.
///
/// The printed channel told them apart by key absence; the recorder brackets `CullReset` and
/// `CullDispatch` unconditionally, so an artifact alone cannot say which leg produced it. This
/// pins that the tables agree on the zone rather than leaving it to two hand-written lists.
#[test]
fn both_legs_read_the_same_shade_zone() {
    assert_eq!(FLAT_READINGS[0].zone, ZONE_SHADE);
    assert_eq!(
        FROXEL_READINGS[2].zone, FLAT_READINGS[0].zone,
        "the flat leg stopped reading the same bracket as the froxel leg, so the two legs are no \
         longer a null experiment over one measurement"
    );
    assert_ne!(
        FROXEL_READINGS[2].label, FLAT_READINGS[0].label,
        "one zone published under one name from both legs would make the floor table report a \
         single statistic where it means two conditions"
    );
}
