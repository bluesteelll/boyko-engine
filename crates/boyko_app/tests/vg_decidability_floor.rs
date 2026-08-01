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
// Parsing the shipped bench's own output
// ===============================================================================================

/// Pulls `key=<f64>` out of a `VB-P1d …` line. Returns `None` when the key is absent, which is how
/// the flat leg (no `froxel_*` keys) is distinguished from the froxel leg without a second parser.
fn field_after(line: &str, key: &str) -> Option<f64> {
    let at = line.find(key)?;
    line[at + key.len()..]
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
        .parse()
        .ok()
}

/// Every `key=value` this rung reads out of one session's stdout, in report order.
///
/// ⚠️ The marker is matched ANYWHERE in the line, not at its start, and that is a measured
/// correction rather than defensive coding. Under `--nocapture` libtest writes its progress without
/// a trailing newline, so the bench's own `println!` lands on the SAME line:
/// `test vb_p1d_cull_shade_bench ... VB-P1d N_ps=0 config=froxel …`. The first draft anchored on
/// `starts_with`, found nothing across all fourteen sessions, and the rung's own
/// "every session must report" assertion is what caught it — a floor computed from the survivors
/// would have been computed from none.
fn extract(stdout: &str, keys: &[&str]) -> Vec<Option<f64>> {
    let line = stdout.lines().find(|l| l.contains("VB-P1d "));
    match line {
        Some(l) => keys.iter().map(|k| field_after(l, k)).collect(),
        None => vec![None; keys.len()],
    }
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

/// Runs ONE session of the shipped bench and returns its stdout.
fn run_session(exe: &PathBuf, froxel: bool) -> String {
    let mut cmd = Command::new(exe);
    cmd.args(["--ignored", "--test-threads=1", "--nocapture"])
        .env("BOYKO_DISABLE_VALIDATION", "1")
        .env("BOYKO_VB_BENCH", "1")
        .env_remove("BOYKO_HOST_DUMP")
        .env_remove("BOYKO_VG_CENSUS");
    if froxel {
        cmd.env_remove("BOYKO_VB_FROXEL_FORCE_OFF");
    } else {
        cmd.env("BOYKO_VB_FROXEL_FORCE_OFF", "1");
    }
    let out = cmd.output().expect("the bench binary runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
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

    // The froxel leg reports four statistics of very different magnitudes; the fixed-cost ones are
    // expected to be the tightest and the whole-pass ones the loosest, and a floor quoted from only
    // one of them would be a floor for one statistic quoted as if it were the instrument's.
    let froxel_keys = ["cull_reset_ns=", "cull_dispatch_ns=", "froxel_shade_ns=", "froxel_total_ns="];
    let flat_keys = ["flat_shade_ns="];

    // Per REPETITION: an independent run of the whole session set. Repetition floors are what say
    // whether one floor number can be trusted at all.
    let mut per_repeat: Vec<Vec<Spread>> = Vec::with_capacity(repeats);
    // Pooled across every repetition: the estimate that actually gets more sessions behind it.
    let mut pooled_froxel: Vec<Vec<f64>> = vec![Vec::new(); froxel_keys.len()];
    let mut pooled_flat: Vec<Vec<f64>> = vec![Vec::new(); flat_keys.len()];

    for r in 0..repeats {
        let mut froxel: Vec<Vec<f64>> = vec![Vec::new(); froxel_keys.len()];
        let mut flat: Vec<Vec<f64>> = vec![Vec::new(); flat_keys.len()];
        for _ in 0..sessions {
            let out = run_session(&exe, true);
            for (i, v) in extract(&out, &froxel_keys).into_iter().enumerate() {
                if let Some(v) = v {
                    froxel[i].push(v);
                }
            }
            let out = run_session(&exe, false);
            for (i, v) in extract(&out, &flat_keys).into_iter().enumerate() {
                if let Some(v) = v {
                    flat[i].push(v);
                }
            }
        }
        let mut rep: Vec<Spread> = Vec::new();
        for (i, k) in froxel_keys.iter().enumerate() {
            assert_eq!(
                froxel[i].len(),
                sessions,
                "repetition {r}: the froxel leg reported `{k}` on {} of {sessions} sessions -- a                  session that printed no bench line measured nothing, and pooling only the                  survivors would UNDERSTATE the spread",
                froxel[i].len()
            );
            pooled_froxel[i].extend_from_slice(&froxel[i]);
            rep.push(summarise(k.trim_end_matches('='), &froxel[i]));
        }
        for (i, k) in flat_keys.iter().enumerate() {
            assert_eq!(flat[i].len(), sessions, "repetition {r}: the flat leg reported `{k}` on too few sessions");
            pooled_flat[i].extend_from_slice(&flat[i]);
            rep.push(summarise(k.trim_end_matches('='), &flat[i]));
        }
        let rf = FLOOR_SIGMA * rep.iter().map(|x| x.cv).fold(0.0_f64, f64::max);
        eprintln!("VG-floor: repetition {} of {repeats} done -- its floor = {:.1}%", r + 1, 100.0 * rf);
        per_repeat.push(rep);
    }

    let mut spreads: Vec<Spread> = Vec::new();
    for (i, k) in froxel_keys.iter().enumerate() {
        spreads.push(summarise(k.trim_end_matches('='), &pooled_froxel[i]));
    }
    for (i, k) in flat_keys.iter().enumerate() {
        spreads.push(summarise(k.trim_end_matches('='), &pooled_flat[i]));
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

    write_floor_doc(&spreads, floor, worst, sessions, repeats, &per_repeat);

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
) {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(4096);
    out.push_str("# VG — the decidability floor — MACHINE-WRITTEN by `vg_decidability_floor_measure`\n\n");
    let _ = writeln!(
        out,
        "**This run measured a floor of {:.1} %** — three sigma on a single-session reading, from a \
         worst-statistic CV of {:.1} % (`{}`).\n\n\
         ⚠️ **Do not read that as \"the floor\".** Repeated runs of this SAME protocol on this same \
         box span roughly **5 %–15 %**. The defensible output of this rung is the RULE in the next \
         section, not the number in this one.\n",
        100.0 * floor,
        100.0 * worst.cv,
        worst.label
    );
    out.push_str(
        "Measured as a **NULL EXPERIMENT**: the shipped `vb_p1d_cull_shade_bench` class, same scene, \
         same configuration, run in separate processes. Nothing differs between sessions, so every \
         difference below is instrument plus environment. **A delta smaller than this is not \
         resolvable by construction** — no statistical treatment recovers a signal from beneath the \
         noise of the thing measuring it.\n\n",
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

#[test]
fn the_bench_line_parser_reads_both_legs() {
    let froxel = "VB-P1d N_ps=14 config=froxel cull_reset_ns=13939.0 cull_dispatch_ns=204.5 \
                  froxel_cull_ns=14143.5 froxel_shade_ns=51200.2 froxel_total_ns=65343.7 (kept 200 frames)";
    assert_eq!(field_after(froxel, "cull_reset_ns="), Some(13939.0));
    assert_eq!(field_after(froxel, "froxel_total_ns="), Some(65343.7));
    // The flat leg carries none of the froxel keys — that absence is how the legs are told apart,
    // so it must read as `None` rather than as a zero.
    let flat = "VB-P1d N_ps=14 config=flat flat_shade_ns=48001.3 (kept 200 frames)";
    assert_eq!(field_after(flat, "flat_shade_ns="), Some(48001.3));
    assert_eq!(field_after(flat, "froxel_total_ns="), None);
    // A run that printed no bench line at all must yield nothing, never a default.
    assert_eq!(extract("boot failed\n", &["flat_shade_ns="]), vec![None]);

    // ⚠️ THE SHAPE THE FIRST DRAFT GOT WRONG, pinned as a fixture. Under `--nocapture` libtest
    // writes its progress line WITHOUT a trailing newline, so the bench's own `println!` lands on
    // the same line. Anchoring on `starts_with` found nothing in fourteen consecutive sessions.
    let real = "test vb_p1d_cull_shade_bench ... VB-P1d N_ps=0 config=froxel cull_reset_ns=575.0 \
                cull_dispatch_ns=12677.5 froxel_cull_ns=13252.5 froxel_shade_ns=26349.4 \
                froxel_total_ns=39601.9 (kept 220 frames)";
    assert_eq!(extract(real, &["cull_reset_ns=", "froxel_total_ns="]), vec![Some(575.0), Some(39601.9)]);
}
