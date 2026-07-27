//! Symbol-reachability gate for the virtual-geometry campaign's two frozen files and the plan
//! that is supposed to consume them.
//!
//! # Why this test exists
//!
//! `docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md` reached Rev 7 with the following revision record, kept
//! in the document itself because the errors are the useful part: Rev 2 claimed four fixes and
//! **one** held; Rev 3 claimed ten and **four** held; Rev 5 claimed eight and **two** held; Rev 6
//! claimed ten and **two** held; Rev 7 claimed four and **one** held. Five consecutive adversarial
//! reviews, five overclaims.
//!
//! Rev 7 named what it believed the cause to be — *"a fix that lands only in the frozen file has
//! not landed"* — because six of Rev 6's ten fixes had been written into
//! `docs/VG-CAMPAIGN-THRESHOLDS.toml` while §8, the section an implementer codes from, kept the
//! superseded rule. The Rev 7 review found that diagnosis to be a **proper subset** of the cause,
//! and the decisive evidence was Rev 7's own edit: the field it added to embody the slogan,
//! `[absolute_mode].absolute_effective_floor_rule`, is orphaned *inside the frozen file* —
//! `absolute_gate_rule` consumes `absolute_floor_ms`, which `absolute_floor_ms_rule` still defines
//! as the superseded spread-only product, so the operative floor in **both** documents is the
//! retired one and the new rule is read by nothing.
//!
//! The accurate generalisation is one level up, and it is what this file mechanises:
//!
//! > **A rule is landed only when some consumer reads the symbol it defines.**
//!
//! Cross-file duplication (§8 ↔ TOML), intra-file symbol binding (`absolute_gate_rule` →
//! `absolute_floor_ms`) and field-to-rung binding (`[pre_registered]`, `min_covered_pixels`, the
//! `[gating]` rows) are one defect wearing three hats. Each of the three has shipped at least once
//! in a revision that believed it had eliminated it. None of them is hard to detect; nothing was
//! looking.
//!
//! # What is checked
//!
//! Three classes, over `docs/VG-CAMPAIGN-THRESHOLDS.toml`, `docs/VG-CAMPAIGN-CLAIM.toml` and the
//! plan:
//!
//! 1. **Dangling citation** — a `[table].field` named in the plan that neither frozen file
//!    defines. A gate pointing at nothing. `[k1].k1_fire_rule` shipped this way and was the
//!    campaign's stated abort criterion for K1.
//! 2. **Unread rule definition** — a key of the form `<sym>_rule` whose defined symbol `<sym>`
//!    appears in no other rule string in the same file, and whose own key the plan never cites.
//!    That is `absolute_effective_floor_rule` exactly: a definition with no consumer on either
//!    side.
//! 3. **Orphan field** — a key defined in a frozen file and named nowhere in the plan. An
//!    implementer coding from §8 would never build it, so its frozen value decides nothing.
//!
//! # What is NOT checked — enumerated, because a gate that does not name its exclusions is the
//! defect this campaign keeps finding
//!
//! * **Semantics.** The sweep is about *reachability of names*, not about whether a rule is
//!   correct, dimensionally sound, or able to go red. Rev 7's review found eight blocking P0s of
//!   which this gate mechanises the detection of three; the other five — a floor with no defined
//!   measurand, a reference floor composed the way `[scope].chain_floor_rule` forbids, a missing
//!   post-fill claim assertion, a self-widening `max()` with its precondition dropped, and a
//!   histogram gate that false-reds a correct instrument — are **arithmetic** defects that no
//!   name-reachability sweep can see. Do not read a green run here as "the plan's gates are
//!   sound".
//! * **Direction of agreement.** When §8 and a frozen file both state a rule, this gate sees two
//!   mentions of a name and is satisfied. It cannot tell that they state *different rules* — which
//!   is precisely what happened to `absolute_floor_source`, where §8 quotes, in the present tense,
//!   the value Rev 7 deleted. Textual agreement is a separate check and is not attempted here.
//! * **Prose citation of a value rather than a field.** §8 frequently restates a formula instead
//!   of naming the field that freezes it (`absolute_floor_ms = spread × measured_chain_median_ms`).
//!   That restatement is invisible to class 3 and is reported as an orphan, which is the intended
//!   reading: a restated rule is a second copy, not a consumer.
//! * **The research document and `docs/VG-R0-REFERENCE-RIG.toml`.** The former states no gate; the
//!   latter does not exist yet (R0a has not run, and no code exists).
//! * **Provenance keys.** [`PROVENANCE_KEYS`] is excluded from class 3 by name — these record
//!   facts about the file itself rather than decision rules, so "the plan never cites it" is not a
//!   defect. The list is short, explicit and asserted non-empty; every other key is in scope.
//!
//! # Why a pinned baseline rather than a hard failure
//!
//! The sweep reports 32 violations on the tree as of this commit, and every one of them is a
//! **true positive** — that is the finding, not a reason to weaken the sweep. Failing outright
//! would make the workspace red for a document that is `NOT APPROVED` and under active revision,
//! and this repository already ruled on that shape once: a legitimate finding must not red a gate
//! and block the ladder (the plan's own R0c(e) disposition, "measure and record here, gate at the
//! next rung").
//!
//! So the baseline is asserted for **exact equality**, not as a ceiling. A new violation reds it,
//! and so does a *fixed* one — which is the point. The census cannot drift in either direction
//! without a deliberate, visible edit in the same commit, and the baseline doubles as the
//! machine-checked worklist a Rev 8 author edits down. A `<=` assertion would let the count fall
//! silently and would not distinguish "repaired" from "the scanner stopped seeing it".
//!
//! # Sensitivity
//!
//! Three controls, because a reachability sweep that cannot see an injected break is vacuous and
//! this file's whole argument is that nothing was watching. Each injects into an **in-memory
//! copy** — no fixture on disk, no committed document touched — and asserts the specific class
//! fires. They are what make the baseline's green mean anything.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Keys excluded from the orphan class because they record facts about the file rather than
/// decision rules: a schema version, an authoring date, the freeze's own start condition. The
/// plan has no reason to cite them and their being uncited is not a defect.
///
/// Deliberately short and deliberately by bare key name (both files carry `schema_version`). Every
/// key not listed here is in scope for class 3 — including every threshold, denominator, direction,
/// rule string and gating row.
const PROVENANCE_KEYS: [&str; 7] = [
    "authored_at_revision",
    "authored_date",
    "freeze_begins_at",
    "frozen_at_revision",
    "frozen_date",
    "pre_r0a_edits_are_authoring_not_amendment",
    "schema_version",
];

/// Class 1 — `[table].field` cited in the plan that no frozen file defines.
///
/// One entry, and it is the campaign's own abort criterion for K1. §9 clause 1 opens by naming
/// `[k1].k1_fire_rule` and then annotates it, in place, as *"a field that does not exist"*. Rev 7
/// caught the danglingness and left the operative sentence naming the dangling field, which is why
/// a reader-facing annotation is not a repair: the name is still what an implementer would grep
/// for. `[k1]` carries `rule`, `k1_decision_rule`, `k1_fire_at_r0` and `k1_fire_instrument_status`.
const BASELINE_DANGLING_CITATIONS: [&str; 1] = ["k1.k1_fire_rule"];

/// Class 2 — `<sym>_rule` keys whose symbol no other rule reads and whose key the plan never
/// cites.
///
/// * `absolute_effective_floor_rule` is the Rev 7 review's blocking P0 B1. Its own comment states
///   it exists because *"without it absolute_gate_rule collapses at s → 0 to `c < m`"* — and
///   `absolute_gate_rule` does not name it.
/// * `histogram_shift_rule` names the per-pair `log2` replacement for the two-bucket constant that
///   was red by construction. §8 R0d(c) states that rule in prose and cites only the tolerance
///   field beside it, so the rule key itself has no consumer.
const BASELINE_UNREAD_RULES: [&str; 2] = [
    "thresholds:absolute_mode.absolute_effective_floor_rule",
    "thresholds:k1_instrument.histogram_shift_rule",
];

/// Class 3 — fields frozen in a companion file that the plan names nowhere.
///
/// The two clusters worth knowing without reading the list: the whole of `[pre_registered]` except
/// `r0e_min_quads` (the table was created at Rev 5 specifically to give two decision-bearing
/// thresholds a file to be registered in, and no rung reads any of them — the Rev 7 review's P0
/// B8), and six of the seven `[gating]` rows, which are the mechanism by which an unanswered owner
/// VALUES call is supposed to block a rung.
const BASELINE_ORPHAN_FIELDS: [&str; 29] = [
    "claim:gating.gating_must_agree_with_hashed_ordering",
    "claim:gating.r0a_blocked_by",
    "claim:gating.r0b_blocked_by",
    "claim:gating.r0c_blocked_by",
    "claim:gating.r0d_blocked_by",
    "claim:gating.r0f_blocked_by",
    "claim:gating.r0f_prime_blocked_by",
    "claim:quality.nanite_max_pixels_per_edge",
    "thresholds:absolute_mode.absolute_distance_ms_rule",
    "thresholds:absolute_mode.absolute_effective_floor_rule",
    "thresholds:absolute_mode.absolute_floor_ms_rule",
    "thresholds:absolute_mode.absolute_gate_red_mutations",
    "thresholds:absolute_mode.absolute_lattice_evidence_required",
    "thresholds:census.readback_retention",
    "thresholds:decidability.reference_floor_source",
    "thresholds:k1.measured_at",
    "thresholds:k1.modal_bucket_pixels_above_which_k1_holds",
    "thresholds:k1.modal_bucket_role",
    "thresholds:k1.report_only",
    "thresholds:k1_instrument.d_est_bound_direction",
    "thresholds:k1_instrument.d_est_may_fire_k1",
    "thresholds:k1_instrument.d_est_may_refute_k1",
    "thresholds:k1_instrument.histogram_shift_excludes_rungs",
    "thresholds:k1_instrument.histogram_shift_rule",
    "thresholds:ordering.claim_blocks_rung_nanite_relative",
    "thresholds:ordering.harness_withholds_floor_while_claim_pending",
    "thresholds:pre_registered.r0c_oracle_coverage_tolerance",
    "thresholds:pre_registered.r0e_ci_max_fraction",
    "thresholds:pre_registered.r0e_min_pairs",
];

/// Lower bound on the fields the parser must recover across both frozen files. A pattern that
/// stopped matching — a reformatted table header, a key style the regex-free scanner does not
/// recognise — would otherwise empty every violation set and report a triumphant green.
const MIN_FIELDS_PARSED: usize = 70;

/// Lower bound on the plan's length in bytes, for the same reason: an empty or moved plan makes
/// every field an orphan and every citation absent, which is a very different failure from a clean
/// document.
const MIN_PLAN_BYTES: usize = 60_000;

/// The repository root — this package's manifest directory IS the workspace root, so no `../..`
/// walking can point the scan at the wrong tree (the `internal_docs_anchors.rs` rationale
/// verbatim).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Drops a `#` comment, respecting `"`-quoted spans so a `#` inside a rule string survives.
///
/// The frozen files carry rule strings with prose in them, and TOML values are the payload this
/// whole gate reads — truncating one at an in-string `#` would silently shrink the set of symbols
/// considered "read" and manufacture unread-rule violations.
fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

/// One `key = value` row with the `[table]` it appeared under.
#[derive(Debug, Clone)]
struct Field {
    table: String,
    key: String,
    value: String,
}

impl Field {
    /// `table.key`, or bare `key` for a top-level row.
    fn dotted(&self) -> String {
        if self.table.is_empty() {
            self.key.clone()
        } else {
            format!("{}.{}", self.table, self.key)
        }
    }
}

/// Recovers every `key = value` row and the table it sits under.
///
/// Hand-written rather than pulled from a TOML crate: this package has zero dependencies by
/// design, which is what lets the gate run with no GPU, no `dxc` and no corpus. The frozen files
/// are flat — `[table]` headers and one `key = value` per line — so the parser only has to be
/// right about comments and headers, and [`MIN_FIELDS_PARSED`] catches it if that stops being
/// true.
fn parse_fields(text: &str) -> Vec<Field> {
    let mut table = String::new();
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']'))
            && !inner.starts_with('[')
            && inner
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            table = inner.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        out.push(Field {
            table: table.clone(),
            key: key.to_string(),
            value: value.trim().to_string(),
        });
    }
    out
}

/// Extracts every `[table].field` citation from the plan.
///
/// Regex-free: scan for `].` and read an identifier on each side. This is the form the plan uses
/// when it points at a frozen value (`[census].decision_resolution`), and it is the form a gate
/// pointing at nothing takes.
fn cited_dotted_fields(plan: &str) -> BTreeSet<String> {
    let bytes = plan.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = BTreeSet::new();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] != b']' || bytes[i + 1] != b'.' {
            continue;
        }
        // table: back over the identifier, then require the opening '['.
        let mut start = i;
        while start > 0 && ident(bytes[start - 1]) {
            start -= 1;
        }
        if start == 0 || bytes[start - 1] != b'[' || start == i {
            continue;
        }
        let table = &plan[start..i];
        // field: forward over the identifier after the '.'.
        let mut end = i + 2;
        while end < bytes.len() && ident(bytes[end]) {
            end += 1;
        }
        if end == i + 2 {
            continue;
        }
        out.insert(format!("{table}.{}", &plan[i + 2..end]));
    }
    out
}

/// The three violation sets, plus the counts the non-vacuity assertions read.
#[derive(Debug, Default)]
struct Report {
    dangling_citations: BTreeSet<String>,
    unread_rules: BTreeSet<String>,
    orphan_fields: BTreeSet<String>,
    fields_parsed: usize,
}

/// Runs the sweep over supplied text, so the sensitivity controls can inject a break without
/// touching a committed document.
fn sweep(thresholds: &str, claim: &str, plan: &str) -> Report {
    let files = [("thresholds", thresholds), ("claim", claim)];
    let parsed: Vec<(&str, Vec<Field>)> = files
        .iter()
        .map(|(tag, text)| (*tag, parse_fields(text)))
        .collect();

    let mut report = Report {
        fields_parsed: parsed.iter().map(|(_, f)| f.len()).sum(),
        ..Default::default()
    };

    // Class 1 — cited by the plan, defined by neither file.
    let defined: BTreeSet<String> = parsed
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(Field::dotted))
        .collect();
    for cited in cited_dotted_fields(plan) {
        if !defined.contains(&cited) {
            report.dangling_citations.insert(cited);
        }
    }

    for (tag, fields) in &parsed {
        for field in fields {
            let dotted = field.dotted();
            let tagged = format!("{tag}:{dotted}");

            // Class 2 — `<sym>_rule` defining a symbol no sibling rule reads.
            if let Some(symbol) = field.key.strip_suffix("_rule") {
                let read_by_sibling = fields
                    .iter()
                    .any(|other| other.key != field.key && other.value.contains(symbol));
                if !read_by_sibling && !plan.contains(&field.key) {
                    report.unread_rules.insert(tagged.clone());
                }
            }

            // Class 3 — defined here, named nowhere in the plan.
            if PROVENANCE_KEYS.contains(&field.key.as_str()) {
                continue;
            }
            if !plan.contains(&field.key) && !plan.contains(&dotted) {
                report.orphan_fields.insert(tagged);
            }
        }
    }
    report
}

fn live_sweep() -> Report {
    sweep(
        &read("docs/VG-CAMPAIGN-THRESHOLDS.toml"),
        &read("docs/VG-CAMPAIGN-CLAIM.toml"),
        &read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md"),
    )
}

fn assert_set_matches(label: &str, actual: &BTreeSet<String>, baseline: &[&str]) {
    let expected: BTreeSet<String> = baseline.iter().map(|s| (*s).to_string()).collect();
    let new: Vec<&String> = actual.difference(&expected).collect();
    let gone: Vec<&String> = expected.difference(actual).collect();
    assert!(
        new.is_empty() && gone.is_empty(),
        "{label}: the symbol-reachability census moved.\n  \
         NEW (a symbol lost its consumer, or a citation lost its definition): {new:?}\n  \
         REPAIRED but still listed in the baseline: {gone:?}\n\
         Both directions are failures by design. A new entry is a defect of the class this gate \
         exists to catch; a repaired entry means the baseline in tests/vg_symbol_reachability.rs \
         must be edited down in the SAME commit as the repair, so the census can never drift \
         quietly in either direction."
    );
}

/// The gate. Every symbol frozen in a companion file has a consumer, or is one of the entries
/// recorded below — and the recorded set is exactly what it was when last blessed.
#[test]
fn every_frozen_symbol_has_a_consumer_or_is_a_recorded_exception() {
    let report = live_sweep();

    // Non-vacuity first: a scanner that recovered nothing would satisfy every set comparison
    // below by returning three empty sets against three empty baselines, if the baselines were
    // ever emptied. Assert the denominators the gate is quantified over.
    assert!(
        report.fields_parsed >= MIN_FIELDS_PARSED,
        "parsed only {} fields from the two frozen files (expected >= {MIN_FIELDS_PARSED}) — the \
         scanner has stopped recognising the file format, and every violation set below is \
         vacuous",
        report.fields_parsed
    );

    assert_set_matches(
        "dangling citations",
        &report.dangling_citations,
        &BASELINE_DANGLING_CITATIONS,
    );
    assert_set_matches("unread rule definitions", &report.unread_rules, &BASELINE_UNREAD_RULES);
    assert_set_matches("orphan fields", &report.orphan_fields, &BASELINE_ORPHAN_FIELDS);

    eprintln!(
        "vg_symbol_reachability: {} fields across two frozen files; {} dangling citations, {} \
         unread rule definitions, {} orphan fields — all matching the recorded baseline.",
        report.fields_parsed,
        report.dangling_citations.len(),
        report.unread_rules.len(),
        report.orphan_fields.len(),
    );
}

/// Non-vacuity for the plan side, asserted separately so a healthy TOML parse cannot mask a plan
/// that moved or emptied. A missing plan makes every field an orphan — a very different failure
/// from a clean document, and one the baseline comparison would report as 40-odd new entries
/// without ever naming the cause.
#[test]
fn the_plan_is_present_and_is_the_document_the_baseline_was_taken_against() {
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");
    assert!(
        plan.len() >= MIN_PLAN_BYTES,
        "the plan is {} bytes (expected >= {MIN_PLAN_BYTES}) — it moved, emptied or was truncated",
        plan.len()
    );
    assert!(
        plan.contains("VG-CAMPAIGN-THRESHOLDS.toml") && plan.contains("VG-CAMPAIGN-CLAIM.toml"),
        "the plan no longer references both frozen files by name; the sweep is scanning the wrong \
         document"
    );
    assert!(
        !PROVENANCE_KEYS.is_empty(),
        "the provenance exclusion list is empty — class 3's exclusions must stay enumerated"
    );
}

/// Sensitivity control for class 1. A citation of a field neither file defines must be reported.
///
/// The injected name is deliberately shaped like a real one (`[decidability].joint_floor_rule` is
/// real; `joint_floor_rule_v2` is not), because the failure this class exists to catch is a rule
/// renamed in one document and left standing in the other — not a typo.
#[test]
fn the_sweep_reports_an_injected_dangling_citation() {
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    let clean = sweep(&thresholds, &claim, &plan);
    let injected = format!("{plan}\n\nThe gate reads `[decidability].joint_floor_rule_v2`.\n");
    let dirty = sweep(&thresholds, &claim, &injected);

    let new: Vec<&String> = dirty
        .dangling_citations
        .difference(&clean.dangling_citations)
        .collect();
    assert_eq!(
        new,
        vec![&"decidability.joint_floor_rule_v2".to_string()],
        "RED: citing a field no frozen file defines was NOT reported. Class 1 is blind, which \
         makes the baseline comparison above vacuously green for the defect that shipped as \
         `[k1].k1_fire_rule`. This is a finding about the scanner — do not retune the injection \
         until it passes."
    );
}

/// Sensitivity control for class 3. A field added to a frozen file that the plan never names must
/// be reported — the shape of every `[pre_registered]` entry.
#[test]
fn the_sweep_reports_an_injected_orphan_field() {
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    let clean = sweep(&thresholds, &claim, &plan);
    let injected = format!("{thresholds}\nan_unconsumed_threshold_for_the_control = 0.5\n");
    let dirty = sweep(&injected, &claim, &plan);

    let new: Vec<&String> = dirty.orphan_fields.difference(&clean.orphan_fields).collect();
    assert!(
        new.iter()
            .any(|s| s.ends_with("an_unconsumed_threshold_for_the_control")),
        "RED: a frozen field the plan never names was NOT reported. Class 3 is blind. new={new:?}"
    );
}

/// Sensitivity control for class 2, and the one that matters most: it reproduces the exact defect
/// the Rev 7 review found in Rev 7's own edit.
///
/// `absolute_gate_rule` consumes `absolute_floor_ms`. Rename the symbol in the consuming rule only
/// — the shape of every "renamed in one place" edit — and `absolute_floor_ms_rule`'s definition
/// must become unread. If it does not, class 2 cannot see an orphaned definition inside a frozen
/// file, which is the failure mode Rev 7 shipped.
#[test]
fn the_sweep_reports_a_rule_whose_only_consumer_stops_naming_it() {
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    assert!(
        thresholds.contains("absolute_floor_ms_rule")
            && thresholds.contains("absolute_gate_rule"),
        "invariant: [absolute_mode] must carry both `absolute_floor_ms_rule` (the definition) and \
         `absolute_gate_rule` (its only consumer) for this control to be meaningful — if the table \
         was restructured, re-derive this test rather than deleting it"
    );

    let clean = sweep(&thresholds, &claim, &plan);
    assert!(
        !clean
            .unread_rules
            .contains("thresholds:absolute_mode.absolute_floor_ms_rule"),
        "invariant: `absolute_floor_ms_rule` is READ today (by `absolute_gate_rule`), which is \
         what makes the mutation below a change"
    );

    // The mutation: the consumer stops naming the symbol. Only `absolute_gate_rule`'s own line is
    // touched, so the definition survives untouched and simply loses its reader.
    let mutated: String = thresholds
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("absolute_gate_rule") {
                line.replace("absolute_floor_ms", "absolute_renamed_floor_ms")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dirty = sweep(&mutated, &claim, &plan);

    assert!(
        dirty
            .unread_rules
            .contains("thresholds:absolute_mode.absolute_floor_ms_rule"),
        "RED: a rule definition left with no consumer inside the frozen file was NOT reported. \
         Class 2 is blind, and this gate would not have caught \
         `absolute_effective_floor_rule` — the defect it was written for. unread={:?}",
        dirty.unread_rules
    );
}
