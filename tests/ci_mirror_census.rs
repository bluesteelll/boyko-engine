//! **The CI mirror cannot rot** (ci-fix-2, orchestrator ruling 96): every job of
//! `.github/workflows/ci.yml` is run by `tools/ci_mirror.py` or named there as not runnable on
//! this host, with a reason.
//!
//! # Why a census and not a convention
//!
//! The mirror is the merge check's copy of CI on the Windows host: GitHub CI run 39663719484 failed
//! 13 of 23 jobs on two defects only the `ubuntu-latest` jobs could see, and no lane had run them. A
//! mirror that silently misses the next job added to ci.yml reproduces that blind spot with a
//! table that reads as complete. So the ordinary test run reads both files and fails on any job,
//! or matrix leg, that the mirror's JOB TABLE does not cover, and on any row with no job. The
//! script checks the same thing at run time (exit 2), and this test makes the drift red where every
//! lane sees it: `cargo test --workspace`.
//!
//! # The two readers, and what they refuse
//!
//! * ci.yml: the job ids are the two-space-indented keys after `jobs:`; each job's `runs-on`; its
//!   `strategy.matrix`, which must be ONE key with a one-line flow list. A matrix with a second
//!   key, `include:`, or a block list is a RED that says "extend the reader", never "no legs" (a
//!   matrix read as having no legs would let every leg go unmirrored).
//! * `tools/ci_mirror.py`: the lines between the `CI-MIRROR JOB TABLE` sentinels that contain a
//!   `|`, each `id | kind | note`.
//!
//! Adding a job to ci.yml = one row in that table. This test names the row to add.
//!
//! # No Linux build by default (ruling 101)
//!
//! The owner banned Linux builds on the dev host (2026-10-22, orchestrator ruling 101): GitHub CI
//! covers the Linux jobs, and the mirror's `linux` and `linux-compile-only` rows run only with
//! `--linux`; without it they are SKIPPED "not run: owner ban (ruling 101) - GitHub CI covers them".
//! [`the_linux_rows_run_only_when_asked`] pins the script's `OPT_IN` table, which its `select()`
//! reads, so a default merge check cannot quietly start building for Linux again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The kinds a row may carry. `tools/ci_mirror.py`'s `KINDS` must say the same, and
/// [`the_mirror_table_rows_are_well_formed`] checks that it does.
const KINDS: [&str; 6] = [
    "script",
    "linux",
    "linux-compile-only",
    "host",
    "host-heavy",
    "not-runnable",
];

/// Kinds that do not run the whole job here, so the row must say why.
const KINDS_NEEDING_A_NOTE: [&str; 3] = ["linux-compile-only", "host-heavy", "not-runnable"];

/// A note shorter than this is not a reason.
const NOTE_MIN: usize = 40;

/// Kinds that run only when their flag is passed, with the flag. Every Linux kind is behind
/// `--linux` (ruling 101). `tools/ci_mirror.py`'s one-line `OPT_IN` must say the same.
const OPT_IN: [(&str, &str); 3] = [
    ("linux", "--linux"),
    ("linux-compile-only", "--linux"),
    ("host-heavy", "--heavy"),
];

/// Floors, not counts: ci.yml had 17 jobs and the table 18 rows when this census landed. A floor
/// cannot stop the job set from shrinking; it stops a reader that sees nothing from passing.
const MIN_JOBS: usize = 17;
const MIN_ROWS: usize = 17;

const TABLE_BEGIN: &str = "# === CI-MIRROR JOB TABLE BEGIN ===";
const TABLE_END: &str = "# === CI-MIRROR JOB TABLE END ===";

fn repo_file(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// One ci.yml job as this census reads it.
#[derive(Debug)]
struct CiJob {
    runs_on: Option<String>,
    /// The matrix values of the job's one matrix key; empty for a job without a matrix.
    legs: Vec<String>,
}

/// One JOB TABLE row.
#[derive(Debug)]
struct Row {
    id: String,
    kind: String,
    note: String,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// The jobs of `ci.yml`, keyed by id. Panics (RED) on a matrix shape this reader does not know.
fn read_ci_jobs(text: &str) -> BTreeMap<String, CiJob> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.trim_end() == "jobs:").expect(
        ".github/workflows/ci.yml has no top-level `jobs:` line -- the census cannot read it",
    );
    let mut jobs: BTreeMap<String, CiJob> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut in_matrix = false;
    for line in &lines[start + 1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indent_of(line);
        if indent == 0 {
            break;
        }
        if indent == 2 {
            let id = trimmed
                .strip_suffix(':')
                .filter(|id| {
                    id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                })
                .unwrap_or_else(|| panic!("ci.yml: `{trimmed}` at job indentation is not a job id -- extend the reader"));
            assert!(!jobs.contains_key(id), "ci.yml: job `{id}` appears twice");
            jobs.insert(
                id.to_owned(),
                CiJob {
                    runs_on: None,
                    legs: Vec::new(),
                },
            );
            current = Some(id.to_owned());
            in_matrix = false;
            continue;
        }
        let Some(id) = current.as_ref() else { continue };
        let job = jobs
            .get_mut(id)
            .expect("invariant: the current job was inserted");
        if indent == 4 {
            in_matrix = false;
            if let Some(v) = trimmed.strip_prefix("runs-on:") {
                job.runs_on = Some(v.trim().to_owned());
            }
            continue;
        }
        if indent == 6 {
            in_matrix = trimmed == "matrix:";
            assert!(
                !trimmed.starts_with("matrix:") || in_matrix,
                "ci.yml job `{id}`: an inline `matrix:` value -- extend the reader"
            );
            continue;
        }
        if in_matrix && indent == 8 {
            let (key, value) = trimmed.split_once(':').unwrap_or_else(|| {
                panic!("ci.yml job `{id}`: matrix line `{trimmed}` -- extend the reader")
            });
            let value = value.trim();
            assert!(
                key != "include" && key != "exclude",
                "ci.yml job `{id}`: a matrix `{key}:` -- extend the reader (and tools/ci_mirror.py's)"
            );
            assert!(
                job.legs.is_empty(),
                "ci.yml job `{id}`: a matrix with more than one key (`{key}`) -- extend the reader \
                 (and tools/ci_mirror.py's); legs read as one key would go unmirrored"
            );
            let list = value
                .strip_prefix('[')
                .and_then(|v| v.strip_suffix(']'))
                .unwrap_or_else(|| {
                    panic!(
                        "ci.yml job `{id}`: matrix `{key}` is not a one-line flow list (`{value}`) -- \
                         extend the reader, never read it as no legs"
                    )
                });
            job.legs = list
                .split(',')
                .map(|v| v.trim().trim_matches('"').to_owned())
                .collect();
            assert!(
                job.legs.iter().all(|l| !l.is_empty()),
                "ci.yml job `{id}`: an empty matrix value in `{value}`"
            );
        }
    }
    jobs
}

/// The JOB TABLE of `tools/ci_mirror.py`.
fn read_table(script: &str) -> Vec<Row> {
    let begin = script
        .find(TABLE_BEGIN)
        .expect("tools/ci_mirror.py has no `CI-MIRROR JOB TABLE BEGIN` sentinel");
    let end = script
        .find(TABLE_END)
        .expect("tools/ci_mirror.py has no `CI-MIRROR JOB TABLE END` sentinel");
    assert!(
        begin < end,
        "tools/ci_mirror.py: the table sentinels are out of order"
    );
    script[begin + TABLE_BEGIN.len()..end]
        .lines()
        .filter(|l| l.contains('|'))
        .map(|l| {
            let mut parts = l.splitn(3, '|').map(str::trim);
            let id = parts.next().unwrap_or_default().to_owned();
            let kind = parts.next().unwrap_or_default().to_owned();
            let note = parts.next().unwrap_or_default().to_owned();
            Row { id, kind, note }
        })
        .collect()
}

fn ci_jobs() -> BTreeMap<String, CiJob> {
    read_ci_jobs(&repo_file(".github/workflows/ci.yml"))
}

fn table() -> Vec<Row> {
    read_table(&repo_file("tools/ci_mirror.py"))
}

/// Anti-vacuity: the two readers see what is known to be there. A reader that sees nothing would
/// make the coverage test below pass over an empty set.
#[test]
fn the_ci_yml_reader_sees_the_jobs_it_must() {
    let jobs = ci_jobs();
    assert!(
        jobs.len() >= MIN_JOBS,
        "the ci.yml reader found {} jobs ({:?}); the floor is {MIN_JOBS}. A job was removed (edit \
         MIN_JOBS with the reason) or the reader broke",
        jobs.len(),
        jobs.keys().collect::<Vec<_>>()
    );
    let check = jobs.get("check").expect("ci.yml has a `check` job");
    assert_eq!(
        check.runs_on.as_deref(),
        Some("ubuntu-latest"),
        "`check` runs on ubuntu-latest"
    );
    let test = jobs.get("test").expect("ci.yml has a `test` job");
    assert_eq!(
        test.runs_on.as_deref(),
        Some("windows-latest"),
        "`test` runs on windows-latest"
    );
    assert_eq!(test.legs, ["debug", "release"], "`test`'s matrix legs");
    let rows = table();
    assert!(
        rows.len() >= MIN_ROWS,
        "tools/ci_mirror.py's JOB TABLE parsed to {} rows; the floor is {MIN_ROWS}",
        rows.len()
    );
}

/// Every ci.yml job is covered by a bare row, or by leg rows covering exactly its matrix values;
/// no row names a job or leg that does not exist; no id appears twice.
#[test]
fn every_ci_job_is_mirrored_or_named_not_runnable() {
    let jobs = ci_jobs();
    let rows = table();
    let mut errors: Vec<String> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for r in &rows {
        if !seen.insert(r.id.as_str()) {
            errors.push(format!(
                "`{}` has two rows in the JOB TABLE -- keep one",
                r.id
            ));
        }
    }
    let ids: BTreeSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    for (id, job) in &jobs {
        let bare = ids.contains(id.as_str());
        let leg_rows: BTreeSet<&str> = ids
            .iter()
            .filter_map(|r| {
                r.strip_prefix(id.as_str())
                    .and_then(|rest| rest.strip_prefix(':'))
            })
            .collect();
        if bare && !leg_rows.is_empty() {
            errors.push(format!(
                "`{id}` has a bare row AND leg rows -- keep one form"
            ));
        } else if !bare && leg_rows.is_empty() {
            errors.push(format!(
                "ci.yml job `{id}` is in no row: add `{id} | <kind> | <note>` to tools/ci_mirror.py's \
                 JOB TABLE (kinds: {}), or `not-runnable` with the reason",
                KINDS.join(", ")
            ));
        } else if !bare {
            let want: BTreeSet<&str> = job.legs.iter().map(String::as_str).collect();
            if want != leg_rows {
                errors.push(format!(
                    "ci.yml job `{id}`'s matrix legs {want:?} are covered by leg rows {leg_rows:?} in \
                     tools/ci_mirror.py -- every leg needs exactly one row"
                ));
            }
        }
    }
    for r in &rows {
        let (job, leg) = match r.id.split_once(':') {
            Some((j, l)) => (j, Some(l)),
            None => (r.id.as_str(), None),
        };
        match (jobs.get(job), leg) {
            (None, _) => errors.push(format!(
                "tools/ci_mirror.py's row `{}` names no ci.yml job -- delete the stale row",
                r.id
            )),
            (Some(j), Some(l)) if !j.legs.iter().any(|x| x == l) => errors.push(format!(
                "tools/ci_mirror.py's row `{}` names a leg ci.yml's `{job}` does not have -- delete \
                 the stale row",
                r.id
            )),
            _ => {}
        }
    }
    assert!(
        errors.is_empty(),
        "the CI mirror does not cover ci.yml:\n  {}",
        errors.join("\n  ")
    );
}

/// Every row's kind is in the closed set, a cross-check kind sits only on an `ubuntu-latest` job,
/// a kind that does not run the whole job says why, and the script's own kind list is this one.
#[test]
fn the_mirror_table_rows_are_well_formed() {
    let jobs = ci_jobs();
    let rows = table();
    let mut errors: Vec<String> = Vec::new();
    for r in &rows {
        if !KINDS.contains(&r.kind.as_str()) {
            errors.push(format!(
                "row `{}`: kind `{}` is not one of {}",
                r.id,
                r.kind,
                KINDS.join(", ")
            ));
        }
        if KINDS_NEEDING_A_NOTE.contains(&r.kind.as_str()) && r.note.chars().count() < NOTE_MIN {
            errors.push(format!(
                "row `{}`: a `{}` row needs a note of >= {NOTE_MIN} characters saying why the job is \
                 not run whole here (note required)",
                r.id, r.kind
            ));
        }
        if r.kind.starts_with("linux") {
            let job = r.id.split(':').next().unwrap_or_default();
            let runs_on = jobs.get(job).and_then(|j| j.runs_on.as_deref());
            if runs_on != Some("ubuntu-latest") {
                errors.push(format!(
                    "row `{}`: a `{}` row cross-checks a Linux job, and `{job}` runs on {runs_on:?}",
                    r.id, r.kind
                ));
            }
        }
    }
    let script = repo_file("tools/ci_mirror.py");
    let line = script
        .lines()
        .find(|l| l.starts_with("KINDS = ("))
        .expect("tools/ci_mirror.py has a `KINDS = (` line");
    let script_kinds: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    if script_kinds != KINDS {
        errors.push(format!(
            "tools/ci_mirror.py's KINDS {script_kinds:?} differ from this census's {KINDS:?} -- edit both"
        ));
    }
    assert!(
        errors.is_empty(),
        "malformed JOB TABLE rows:\n  {}",
        errors.join("\n  ")
    );
}

/// Ruling 101: nothing is built for Linux unless asked. Every Linux kind is opt-in behind
/// `--linux`, and the script's `OPT_IN` table (what its `select()` reads) is this census's.
#[test]
fn the_linux_rows_run_only_when_asked() {
    let mut errors: Vec<String> = Vec::new();
    for kind in KINDS.iter().filter(|k| k.starts_with("linux")) {
        if !OPT_IN.contains(&(*kind, "--linux")) {
            errors.push(format!(
                "kind `{kind}` builds for Linux and is not behind `--linux` in this census's OPT_IN \
                 (owner ban, ruling 101)"
            ));
        }
    }
    let script = repo_file("tools/ci_mirror.py");
    let line = script
        .lines()
        .find(|l| l.starts_with("OPT_IN = {"))
        .expect("tools/ci_mirror.py has a one-line `OPT_IN = {` table");
    let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    let script_opt_in: Vec<(&str, &str)> = quoted
        .chunks(2)
        .map(|pair| (pair[0], pair.get(1).copied().unwrap_or_default()))
        .collect();
    if script_opt_in != OPT_IN {
        errors.push(format!(
            "tools/ci_mirror.py's OPT_IN {script_opt_in:?} differs from this census's {OPT_IN:?}: a \
             Linux row must run only with --linux (owner ban, ruling 101) -- edit both, or neither"
        ));
    }
    assert!(
        errors.is_empty(),
        "the CI mirror's opt-in table:\n  {}",
        errors.join("\n  ")
    );
}
