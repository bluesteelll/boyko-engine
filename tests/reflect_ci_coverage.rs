//! **Reflection GATES — D3 C6's named list (landed at G1), and G4's workflow gate (to
//! come).**
//!
//! # The split, recorded where it lives
//!
//! The plan lands this file at G4 (its ci.yml-parsing assertions). Its **named list**
//! lands at G1 by necessity, not convenience — the same shape as G0's recorded
//! amendments: G1's manifest census clause C6 asserts that every command line under
//! `.github/`, `scripts/` and `docs/REFLECTION-PLAN-GATES.md` that enables a `reflect`
//! feature appears in THIS file's named list, and the plan document itself already
//! carries such command lines (its own G0 gate lines and G4 job specifications), so C6
//! has a non-empty subject three rungs before the CI legs exist. A C6 with no reference
//! list would red G1 for a reason G4 owns; a C6 that skipped a missing list would be a
//! gate that cannot fail. The G4 half of this file — parsing `.github/workflows/ci.yml`
//! and asserting the reflect legs exist as specified — arrives with G4 and asserts the
//! REVERSE direction: every `leg`-classified row below is a real CI leg.
//!
//! # What a row means
//!
//! A row is a `--features` SPEC (the token after `--features`, one comma-part at a
//! time) that may legitimately enable a `reflect` feature somewhere in the scan scope.
//! A spec found by G1's scan that has no row here reds the manifest census — that is
//! F17's counter-clause (`hwrt` has feature-gated bodies compiled by NO CI leg,
//! measured `grep -c hwrt ci.yml` = 0; `reflect` is not allowed to inherit that).

/// The named rows. Parsed BY SOURCE TEXT by `tests/reflect_manifest_census.rs` (C6), so
/// the delimiter comments are load-bearing — do not remove or rename them.
///
/// * `"reflect"` — single-package invocations (`cargo check -p reflect-fixture
///   --all-targets --features reflect`, and the dogfood twin) — GATES G0 gate 2. Also
///   the spelling in D4's documented HARD-ERROR example (`-p boyko-reflect --features
///   reflect`), which exists in the plan text precisely as the invocation that cannot
///   run.
/// * `"reflect-fixture/reflect"` — the multi-package `pkg/feature` spelling the CI
///   `reflect-on` job and the Miri sweep use (GATES G4 items 1 and 3; the bare form is
///   ambiguous across a multi-package selection and silently selects nothing).
/// * `"reflect-dogfood/reflect"` — the dogfood job's umbrella spelling (GATES G4 item
///   4; D15).
// BEGIN REFLECT ENABLING SPECS
pub const NAMED_ENABLING_SPECS: &[&str] = &[
    "reflect",
    "reflect-fixture/reflect",
    "reflect-dogfood/reflect",
];
// END REFLECT ENABLING SPECS

/// The C6 scanner, shared with `tests/reflect_manifest_census.rs` (G1's half of the
/// clause) so the two directions run over the same scan.
#[path = "reflect_scan_support/mod.rs"]
mod support;

use std::collections::BTreeSet;

/// Reads `.github/workflows/ci.yml`, normalized to `\n` — the checkout may carry CRLF
/// on this platform (measured: it does), and the block parser keys on newlines.
fn ci_yml() -> String {
    let path = support::repo_root().join(".github").join("workflows").join("ci.yml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e} -- the CI gate has no subject", path.display()))
        .replace("\r\n", "\n")
}

/// Extracts one job's block: from its two-space-indented `  name:` header to the next
/// job header. Panics when the job does not exist — a missing leg is this gate's
/// primary red, never a skip.
fn job_block(yml: &str, job: &str) -> String {
    let needle = format!("\n  {job}:\n");
    let start = yml
        .find(&needle)
        .unwrap_or_else(|| panic!(".github/workflows/ci.yml has no `{job}` job -- the leg this gate pins does not exist"));
    let body = &yml[start + needle.len()..];
    let mut end = body.len();
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let is_job_header = line.len() > 2
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && !line[2..].starts_with('#')
            && line.trim_end().ends_with(':');
        if is_job_header {
            end = offset;
            break;
        }
        offset += line.len();
    }
    body[..end].to_owned()
}

/// A job's non-comment text — for negative assertions, so a comment may still NAME what
/// the job deliberately omits without tripping them.
fn without_comments(block: &str) -> String {
    block.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join("\n")
}

/// **G4 item 5a** — the `reflect-on` job exists, selects both packages, carries the
/// `pkg/feature` spelling, the `[debug, release]` matrix (D7), and the non-vacuity
/// marker its per-test assertion keys on.
#[test]
fn reflect_on_job_is_wired() {
    let job = job_block(&ci_yml(), "reflect-on");
    for needed in [
        "--features reflect-fixture/reflect",
        "-p boyko-reflect",
        "-p reflect-fixture",
        "profile: [debug, release]",
        "BOYKO_REFLECT_LEG: reflect-on",
        "--no-fail-fast",
    ] {
        assert!(
            job.contains(needed),
            "the `reflect-on` job lost `{needed}` -- without it the leg is either \
             vacuous (feature off, marker gone) or half-blind (one profile, fail-fast \
             shadowing)"
        );
    }
}

/// **G4 item 5b** — the dogfood job exists with its umbrella spelling. It is the only CI
/// leg that compiles an engine crate's `reflect` feature (D15), so losing it un-compiles
/// every gated body in `boyko_scene`/`boyko_render` — F17's exact defect.
#[test]
fn reflect_dogfood_job_is_wired() {
    let job = job_block(&ci_yml(), "reflect-dogfood");
    for needed in ["-p reflect-dogfood", "--features reflect-dogfood/reflect", "--no-fail-fast"] {
        assert!(job.contains(needed), "the `reflect-dogfood` job lost `{needed}`");
    }
}

/// **G4 item 5c** — the Miri sweep's two reflect rows keep their two DIFFERENT shapes
/// (D4): `-p boyko-reflect` plain (a feature flag on it is a hard cargo error — the
/// unrunnable sentence four sibling documents once inherited), `-p reflect-fixture` with
/// the `pkg/feature` spelling, and no `reflect-dogfood` anywhere outside comments (Miri
/// cannot execute FFI, F18).
#[test]
fn miri_sweep_names_the_right_rows_in_the_right_shapes() {
    let job = job_block(&ci_yml(), "miri");
    let code = without_comments(&job);
    assert!(
        code.contains("-p boyko-reflect"),
        "the Miri sweep lost `-p boyko-reflect` -- the crate's arithmetic and registry \
         run under Miri through this row (B.9)"
    );
    let reflect_row = code
        .lines()
        .find(|l| l.contains("-p boyko-reflect"))
        .expect("asserted present above");
    assert!(
        !reflect_row.contains("--features"),
        "the Miri sweep's `-p boyko-reflect` row carries a feature flag -- the crate has \
         no `reflect` feature (D4) and this exact line is a hard cargo error: `none of \
         the selected packages contains these features`"
    );
    assert!(
        code.contains("-p reflect-fixture --features reflect-fixture/reflect"),
        "the Miri sweep lost `-p reflect-fixture --features reflect-fixture/reflect` -- \
         the ONLY row that reaches derive-generated unsafe under Miri (B.9); dropping it \
         is the silent revert this gate exists to catch"
    );
    assert!(
        !code.contains("reflect-dogfood"),
        "the Miri sweep names `reflect-dogfood`, which reaches boyko_render -> \
         boyko_rhi_vulkan -- Miri cannot execute FFI (F18), so this row faults for a \
         non-reflection reason"
    );
}

/// **G4 item 5d** — the census job requests `llvm-tools` (D6: tool absence is a RED,
/// and the job must not be a machine on which the census panics for tool reasons).
#[test]
fn reflect_census_job_requests_llvm_tools() {
    let job = job_block(&ci_yml(), "reflect-census");
    assert!(
        job.contains("components: llvm-tools"),
        "the `reflect-census` job does not request llvm-tools -- the census panics \
         without llvm-nm (D6), so this job would red for tool reasons on every run"
    );
    assert!(
        job.contains("--test reflect_absence_census"),
        "the `reflect-census` job does not run the absence census"
    );
}

/// **D3 C6's other half** — the named rows equal the set the shared scan finds, in BOTH
/// directions: an enabling command line with no row reds (G1's census also catches
/// that); a row whose command line vanished reds HERE, so the allowance list cannot
/// outlive the legs it allows.
#[test]
fn named_rows_equal_the_found_enabling_set() {
    let found: BTreeSet<String> = support::scan_scope().into_iter().map(|e| e.spec).collect();
    let named: BTreeSet<String> = NAMED_ENABLING_SPECS.iter().map(|s| (*s).to_owned()).collect();
    let unnamed: Vec<&String> = found.difference(&named).collect();
    let dead: Vec<&String> = named.difference(&found).collect();
    assert!(
        unnamed.is_empty() && dead.is_empty(),
        "D3 C6's equality is broken.\n  found-but-unnamed specs: {unnamed:?}\n  \
         named-but-found-nowhere specs: {dead:?}\nThe named list \
         (NAMED_ENABLING_SPECS above) and the scan scope (.github/, scripts/, the GATES \
         plan) must agree exactly -- a subset check in either direction is how a \
         coverage list rots into decoration"
    );
}

/// Integrity of the named list itself: non-empty, deduplicated, and every row actually
/// names a `reflect` feature — a row that names something else would widen the allowance
/// C6 grants without anyone deciding to.
#[test]
fn named_list_is_well_formed() {
    assert!(
        !NAMED_ENABLING_SPECS.is_empty(),
        "the named list is empty -- C6 would then red every enabling invocation, \
         including the legitimate ones the plan itself specifies"
    );
    for (i, spec) in NAMED_ENABLING_SPECS.iter().enumerate() {
        let is_reflect =
            *spec == "reflect" || spec.split_once('/').is_some_and(|(_, f)| f == "reflect");
        assert!(
            is_reflect,
            "named row `{spec}` does not name a `reflect` feature -- a row here is an \
             allowance, and an allowance for something else is a defect"
        );
        assert!(
            !NAMED_ENABLING_SPECS[..i].contains(spec),
            "named row `{spec}` is duplicated"
        );
    }
}
