//! Pins the owner's release-CI decisions of 2026-10-10 (release PR #4) about where the Miri
//! sweep runs and how the KE16 Miri gates run. No other gate in the tree held them. MEASURED on
//! `ae9c1dc3` by mutation: adding `pull_request:` to `miri-sweep.yml`'s triggers, or putting
//! round 6's well-formed `cfg_attr(miri, ignore = …)` skip back on a KE16 gate, left every root
//! census green (`reflect_ci_coverage`, `reflect_manifest_census`, and `ignore_reasons_census`,
//! which re-counts sites but pins no total).
//!
//! * The pure-compute Miri sweep is NOT a merge gate. Its workflow runs on a weekly `schedule`
//!   and on `workflow_dispatch`, never on `push` or `pull_request`.
//! * The four KE16 Miri gates run under Miri. Round 6 had skipped them (`922f9e47`), round 7
//!   reverted that, and their headers' "deliberately NOT ignored" notes stand. So no code line in
//!   their files may say `ignore`.
//! * The sweep runs those four gates in steps of their own, under the recipe each file's `# Run`
//!   block documents, and its generic step skips exactly their four names. A `--skip` without
//!   that step would bring round 6's skip back through the workflow instead of through an
//!   attribute, so this census ties each skip to a recipe step.
//!
//! Text parsing, not a YAML crate: the root package carries no YAML dependency, and every property
//! here is line-shaped. Each check carries its own anti-vacuity clause.

use std::path::PathBuf;

/// The sweep's workflow, relative to the repository root.
const MIRI_SWEEP_YML: &str = ".github/workflows/miri-sweep.yml";

/// The four KE16 Miri gates: each file, its `--test` target name, and the `#[test]` count it
/// must keep.
const KE16_MIRI_GATES: &[(&str, &str, usize)] = &[
    (
        "crates/boyko_threadpool/tests/miri_scope_completion_protector.rs",
        "miri_scope_completion_protector",
        3,
    ),
    (
        "crates/boyko_threadpool/tests/miri_scope_free_window.rs",
        "miri_scope_free_window",
        1,
    ),
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a repository file, normalized to `\n` (the checkout may carry CRLF).
fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e} -- this census has no subject",
                path.display()
            )
        })
        .replace("\r\n", "\n")
}

/// A file's lines that are not `//` comments, numbered from 1.
fn code_lines(src: &str) -> Vec<(usize, &str)> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .map(|(i, l)| (i + 1, l))
        .collect()
}

/// The trigger names of a workflow's top-level `on:` block, in file order. Panics on an inline
/// `on:` value (`on: [push]`), which this parser does not read and must not pass.
fn triggers(rel: &str) -> Vec<String> {
    let yml = read(rel);
    let mut lines = yml.lines();
    let header = lines
        .by_ref()
        .find(|l| l.starts_with("on:"))
        .unwrap_or_else(|| panic!("{rel} has no top-level `on:` key"));
    assert_eq!(
        header.trim_end(),
        "on:",
        "{rel}: `on:` must be a block, not an inline value: {header:?}"
    );
    let mut out = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            break; // the next top-level key ends the block
        }
        if line.starts_with("  ") && !line.starts_with("   ") {
            let key = trimmed
                .split(':')
                .next()
                .expect("invariant: split yields one item");
            out.push(key.to_owned());
        }
    }
    out
}

/// The `#[test]` fn names of a test file, in file order.
fn test_names(rel: &str) -> Vec<String> {
    let src = read(rel);
    let code = code_lines(&src);
    let mut names = Vec::new();
    for (k, (_, line)) in code.iter().enumerate() {
        if line.trim() != "#[test]" {
            continue;
        }
        let name = code[k + 1..]
            .iter()
            .map(|(_, l)| l.trim())
            .find(|l| !l.starts_with("#["))
            .and_then(|l| l.strip_prefix("fn "))
            .and_then(|l| l.split('(').next())
            .unwrap_or_else(|| {
                panic!("{rel}: a `#[test]` is not followed by a `fn` this census can read")
            });
        names.push(name.to_owned());
    }
    names
}

/// The flags of the first `MIRIFLAGS="…"` in a test file's `# Run` block, in order. The value may
/// span several `//!` lines joined by a trailing `\`.
fn run_block_miriflags(rel: &str) -> Vec<String> {
    let src = read(rel);
    let run = src.find("//! # Run\n").unwrap_or_else(|| {
        panic!("{rel} has no `# Run` block -- the recipe this census compares against")
    });
    let after = &src[run..];
    let open = after
        .find("MIRIFLAGS=\"")
        .unwrap_or_else(|| panic!("{rel}: its `# Run` block spells no `MIRIFLAGS=\"…\"`"));
    let value = &after[open + "MIRIFLAGS=\"".len()..];
    let close = value
        .find('"')
        .unwrap_or_else(|| panic!("{rel}: the `# Run` MIRIFLAGS never closes"));
    value[..close]
        .split_whitespace()
        .filter(|t| *t != "//!" && *t != "\\")
        .map(str::to_owned)
        .collect()
}

/// The steps of `miri-sweep.yml`'s only job, each as its non-comment text. A step starts at a
/// six-space `- ` item under `steps:`.
fn sweep_steps() -> Vec<String> {
    let yml = read(MIRI_SWEEP_YML);
    let start = yml
        .find("\n    steps:\n")
        .unwrap_or_else(|| panic!("{MIRI_SWEEP_YML} has no `steps:` block"));
    let mut steps: Vec<String> = Vec::new();
    for line in yml[start..].lines().skip(2) {
        if line.trim_start().starts_with('#') {
            continue;
        }
        if line.starts_with("      - ") {
            steps.push(String::new());
        }
        if let Some(step) = steps.last_mut() {
            step.push_str(line);
            step.push('\n');
        }
    }
    steps
}

/// A step's `MIRIFLAGS: "…"` value, split into flags.
fn step_miriflags(step: &str) -> Option<Vec<String>> {
    let line = step
        .lines()
        .find(|l| l.trim_start().starts_with("MIRIFLAGS:"))?;
    let value = line
        .trim_start()
        .trim_start_matches("MIRIFLAGS:")
        .trim()
        .trim_matches('"');
    Some(value.split_whitespace().map(str::to_owned).collect())
}

/// Decision (1): the sweep's workflow triggers on exactly `schedule` and `workflow_dispatch`.
/// Anti-vacuity: the same parser must find `push` and `pull_request` in `ci.yml`, which has them.
#[test]
fn miri_sweep_runs_only_on_schedule_and_workflow_dispatch() {
    let ci = triggers(".github/workflows/ci.yml");
    assert!(
        ci.iter().any(|t| t == "push") && ci.iter().any(|t| t == "pull_request"),
        "the trigger parser found {ci:?} in ci.yml, which runs on push and pull_request -- the \
         parser is blind, so the assertion below would prove nothing"
    );
    let sweep = triggers(MIRI_SWEEP_YML);
    assert_eq!(
        sweep,
        ["schedule", "workflow_dispatch"],
        "miri-sweep.yml must run weekly and by hand only (owner decision, 2026-10-10): it costs \
         ~3-3.5 h sequential and is NOT a merge gate, so a push / pull_request trigger would hold \
         every merge for hours. Its header says why and what finishing it takes."
    );
}

/// Decision (2): no code line of either file says `ignore`: no `#[ignore]`, and no
/// `cfg_attr(miri, ignore …)` in any spelling. Comments may still name the attribute (the
/// free-window header's "deliberately NOT `cfg_attr(miri, ignore)`d" does). Anti-vacuity: the
/// `#[test]` count, so a renamed or emptied file cannot pass by having nothing to scan.
#[test]
fn ke16_miri_gates_are_not_ignored() {
    for &(rel, _, want_tests) in KE16_MIRI_GATES {
        let src = read(rel);
        let code = code_lines(&src);
        let tests = code.iter().filter(|(_, l)| l.trim() == "#[test]").count();
        assert_eq!(
            tests, want_tests,
            "{rel}: {tests} `#[test]` fns where {want_tests} are pinned -- a KE16 gate was added or \
             removed; re-read owner decision (2) of 2026-10-10 before moving this count"
        );
        let hits: Vec<String> = code
            .iter()
            .filter(|(_, l)| l.contains("ignore"))
            .map(|(n, l)| format!("{rel}:{n}: {}", l.trim()))
            .collect();
        assert!(
            hits.is_empty(),
            "a KE16 Miri gate is ignored again -- owner decision (2) of 2026-10-10 reverted round \
             6's skips (`922f9e47`), and these gates run under Miri exactly as before:\n{}",
            hits.join("\n")
        );
    }
}

/// Decision (1), round 8: the sweep's `--skip`s name exactly the four KE16 gates, as whole names
/// (`--exact`), and each gate file is run by a step of its own whose `MIRIFLAGS` are the flags
/// its `# Run` block spells. Anti-vacuity: the names come from the files (four of them), and each
/// recipe must carry `-Zmiri-tree-borrows`, the model the gates hunt in.
#[test]
fn ke16_gates_skipped_by_the_sweep_run_under_their_own_recipes() {
    let steps = sweep_steps();
    assert!(
        steps.len() >= 3,
        "{MIRI_SWEEP_YML}: found {} steps -- the step parser is blind",
        steps.len()
    );

    let mut gate_names: Vec<String> = KE16_MIRI_GATES
        .iter()
        .flat_map(|(rel, _, _)| test_names(rel))
        .collect();
    gate_names.sort();
    assert_eq!(
        gate_names.len(),
        4,
        "the KE16 gate files name {gate_names:?}, not four tests"
    );

    let skipping: Vec<&String> = steps.iter().filter(|s| s.contains("--skip ")).collect();
    assert_eq!(
        skipping.len(),
        1,
        "{MIRI_SWEEP_YML}: {} steps carry `--skip`, where exactly one must: the generic sweep \
         step, which skips the KE16 gates that their recipe steps run",
        skipping.len()
    );
    let generic = skipping[0];
    let tokens: Vec<&str> = generic.split_whitespace().collect();
    let mut skipped: Vec<String> = tokens
        .windows(2)
        .filter(|w| w[0] == "--skip")
        .map(|w| w[1].to_owned())
        .collect();
    skipped.sort();
    assert_eq!(
        skipped, gate_names,
        "the sweep's generic step must skip exactly the four KE16 gates, which run in steps of \
         their own (owner decision, 2026-10-10) -- a skip of anything else is an ignore by another \
         name, and an unskipped gate runs under flags that cannot arm it"
    );
    assert!(
        tokens.contains(&"--exact"),
        "the sweep's generic step skips without `--exact`, so each `--skip` is a substring filter \
         and can take out tests it does not name"
    );

    for &(rel, target, _) in KE16_MIRI_GATES {
        let recipe = run_block_miriflags(rel);
        assert!(
            recipe.iter().any(|f| f == "-Zmiri-tree-borrows"),
            "{rel}: the `# Run` MIRIFLAGS parsed as {recipe:?}, without `-Zmiri-tree-borrows` -- \
             the recipe parser is blind, or the recipe lost the model its gates hunt in"
        );
        let needle = format!("--test {target} ");
        let runners: Vec<&String> = steps.iter().filter(|s| s.contains(&needle)).collect();
        assert_eq!(
            runners.len(),
            1,
            "{MIRI_SWEEP_YML}: {} steps run `--test {target}`; the KE16 gate needs exactly one, \
             under its own recipe -- without it the generic step's `--skip` leaves it run nowhere",
            runners.len()
        );
        let flags = step_miriflags(runners[0]).unwrap_or_else(|| {
            panic!("{MIRI_SWEEP_YML}: the `--test {target}` step sets no MIRIFLAGS")
        });
        assert_eq!(
            flags, recipe,
            "{MIRI_SWEEP_YML}: the `--test {target}` step's MIRIFLAGS differ from {rel}'s `# Run` \
             recipe -- the gate is armed only under its own flags"
        );
    }
}
