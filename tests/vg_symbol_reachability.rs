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
//! Four classes, over `docs/VG-CAMPAIGN-THRESHOLDS.toml`, `docs/VG-CAMPAIGN-CLAIM.toml` and the
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
//! 4. **Unresolvable `[gating]` payload** — a `table.field` path *inside* a gating row that no
//!    frozen file defines. Classes 1–3 all test KEYS; this one tests a VALUE, because the gating
//!    rows are the mechanism by which an unanswered owner VALUES call blocks a rung, and a
//!    one-letter typo in one silently unblocks it.
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
//! # The baseline, and what it measured
//!
//! When this gate was written against Rev 7 it reported **32 violations** — 1 dangling citation,
//! 2 unread rule definitions, 29 orphan fields — and every one was a true positive. Twenty of the
//! thirty-two sat in the comparative-claim machinery: the whole `[gating]` table, the whole of
//! `[pre_registered]` bar one field, and the three `[k1_instrument]` fields carrying the campaign's
//! single most important correction (`d_est_bound_direction`, `d_est_may_fire_k1`,
//! `d_est_may_refute_k1` — frozen as data *"so no later rung re-derives them from prose"*, and then
//! named nowhere in the prose). That distribution is what the owner's re-scope decision was taken
//! on.
//!
//! **Rev 8 took all three classes to zero**, by removing the apparatus whose symbols nothing could
//! read at this rung and by making the surviving rungs cite the fields they consume.
//!
//! The baselines below are asserted for **exact equality**, not as ceilings. A new violation reds
//! the gate, and so does a *repaired* one — which is the point: the census cannot drift in either
//! direction without a deliberate edit in the same commit. A `<=` assertion would let the count
//! fall silently and could not distinguish "repaired" from "the scanner stopped seeing it". Now
//! that the baselines are empty, the equality also means the ratchet cannot slip back: any regression
//! is a new entry.
//!
//! # Sensitivity
//!
//! Six controls, because a reachability sweep that cannot see an injected break is vacuous, and
//! with every baseline now empty they are the *only* thing standing between a green run and a
//! scanner that has quietly stopped scanning. Each injects into an **in-memory copy** — no fixture
//! on disk, no committed document touched — and asserts the specific class fires.
//!
//! One of them has already earned its place twice. The class-2 control originally stood on the live
//! `[absolute_mode]` table; Rev 8 removed that table and the control's own invariant assertion
//! caught it, so it now drives a synthetic pair instead — a control that breaks every time the
//! guarded document is restructured is a control that eventually gets deleted rather than
//! re-derived. And the citation control ([`a_bare_english_word_key_is_not_counted_as_a_citation`])
//! found a real false negative in this file's first version, then found a second one in its repair:
//! the plan writes `[table].field` with brackets, so a check for the bare `table.field` matched
//! nothing.

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

/// Class 1 — `[table].field` cited in the plan that no frozen file defines. **Empty at Rev 8.**
///
/// It held one entry, and it was the campaign's own abort criterion for K1: §9 clause 1 opened by
/// naming `[k1].k1_fire_rule` and then annotated it, in place, as *"a field that does not exist"*.
/// Rev 7 caught the danglingness and left the operative sentence spelling the dangling name, which
/// is why a reader-facing annotation is not a repair — the name is still what an implementer greps
/// for. Rev 8's correction note describes the dead field instead of quoting it in citation form,
/// which is the honest way to keep the history: this scanner cannot tell a live citation from a
/// historical one, so leaving the spelling would have cost a permanent exception for a name nothing
/// should resolve.
const BASELINE_DANGLING_CITATIONS: [&str; 0] = [];

/// Class 2 — `<sym>_rule` keys whose symbol no other rule reads and whose key the plan never
/// cites. **Empty at Rev 8.**
///
/// It held two. `absolute_effective_floor_rule` was the Rev 7 review's blocking P0: its own comment
/// stated it existed because *"without it absolute_gate_rule collapses at s → 0 to `c < m`"*, and
/// `absolute_gate_rule` never named it, so the operative floor stayed the superseded spread-only
/// product in both documents. It left with `[absolute_mode]`. `histogram_shift_rule` — the per-pair
/// `log2` replacement for the two-bucket constant that was red by construction — is now cited by
/// §8 R0d, which is also where that check was demoted from a gate to a recorded residual.
const BASELINE_UNREAD_RULES: [&str; 0] = [];

/// Class 3 — fields frozen in a companion file that the plan names nowhere. **Empty at Rev 8.**
///
/// It held 29. Two clusters carried most of it: the whole of `[pre_registered]` bar one field — a
/// table created specifically so two decision-bearing thresholds had a file to be registered in,
/// and no rung ever read any of them — and six of the seven `[gating]` rows, which are the
/// mechanism by which an unanswered owner VALUES call is supposed to block a rung. Both clusters
/// were frozen values that decided nothing.
///
/// They cleared two ways, and the distinction matters for reading a zero here. Twenty left the
/// documents with the decidability apparatus, because no rung of R0 could ever have read them. The
/// other nine were repaired the other way round: the surviving rungs now cite the fields they
/// actually consume — R0c names its pre-registered oracle tolerance and the non-degeneracy floors,
/// R0d names the histogram-shift rule and the report-only statistics, §0.2 names the freeze
/// tripwire's own fields, §9 names the three `d_est_*` direction fields.
const BASELINE_ORPHAN_FIELDS: [&str; 0] = [];

/// The table names the plan is allowed to cite in the bare `table.field` spelling.
///
/// ⚠️ **This list is deliberately hand-maintained and deliberately NOT derived from the files under
/// test, and that is the whole point.** The first attempt at the Rev 9 repair built the set of
/// citable tables by parsing the frozen files — which is circular: deleting a table's last field
/// also removes the table from the derived set, so the citation of the deleted field stops being
/// recognised as a citation and no dangling report appears. The control
/// [`the_sweep_reports_the_deletion_of_a_rung_blocking_field`] failed on exactly that and is why
/// this list exists.
///
/// Both directions are asserted, so the list cannot rot: every table the frozen files define must
/// appear here (a new table is noticed), and a table legitimately removed must be struck from this
/// list in the same commit as its removal — the same ratchet the violation baselines use.
/// It lists **every** table both frozen files define — no judgement about which ones the plan
/// happens to cite bracketed today, because that is exactly the kind of incidental fact that goes
/// stale between revisions.
const CITABLE_TABLES: [&str; 9] = [
    "census",
    "corpus",
    "gating",
    "hash_assertion",
    "k1",
    "k1_instrument",
    "k1_outcome",
    "k2_probe",
    "pre_registered",
];

/// Lower bound on the fields the parser must recover across both frozen files. A pattern that
/// stopped matching — a reformatted table header, a key style the regex-free scanner does not
/// recognise — would otherwise empty every violation set and report a triumphant green.
///
/// Stood at 70 against 77 parsed fields until Rev 8, which removed the decidability apparatus from
/// both files and took the count to 44. The floor fired on that edit, which is correct behaviour
/// and worth recording: this guard cannot tell "the scanner broke" from "the documents legitimately
/// shrank", so it demands a deliberate update either way. Lowering it is part of the same act as
/// the removal, never a reaction to a red run.
const MIN_FIELDS_PARSED: usize = 40;

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
/// Both spellings, and the second one is the Rev 9 repair.
///
/// ⚠️ **This function recognised only `[table].field` until Rev 9, and the omission was
/// load-bearing.** The plan cites `corpus.arrangement` and `k1_outcome.undecided_disposition` —
/// the two fields that block a rung — **without** brackets. Neither was ever in the citation set,
/// so deleting either table from the claim file was undetectable: `defined` would lose the field,
/// but nothing had cited it, so no dangling citation appeared; and class 3 iterates the fields that
/// exist, so a deleted field cannot be an orphan either. The gate would have stayed green with
/// §9's outcome table and `[gating].r1_blocked_by` both naming a field that no longer existed. The
/// asymmetry was the tell: [`is_cited`] already checked both spellings, so the two-spelling problem
/// was solved in one class and not the other, and the class-1 control injected only the bracketed
/// form.
///
/// The bare form is accepted **only when the table name is one the frozen files actually define**.
/// Without that restriction every `file.rs`, `mesh_assets.rs:252` and `self.field` in the prose
/// becomes a citation of a nonexistent table, and the class fills with noise — which is its own
/// way of going blind.
fn cited_dotted_fields(plan: &str, known_tables: &BTreeSet<String>) -> BTreeSet<String> {
    let bytes = plan.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = BTreeSet::new();
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] != b'.' {
            continue;
        }
        // The table name, in whichever of the two spellings precedes this dot.
        let table = if bytes[i - 1] == b']' {
            // `[table].field` — walk back over the name, require the opening bracket.
            let mut s = i - 1;
            while s > 0 && ident(bytes[s - 1]) {
                s -= 1;
            }
            if s == 0 || s == i - 1 || bytes[s - 1] != b'[' {
                continue;
            }
            &plan[s..i - 1]
        } else {
            // `table.field` — accepted only for a table the frozen files define.
            let mut s = i;
            while s > 0 && ident(bytes[s - 1]) {
                s -= 1;
            }
            if s == i || !known_tables.contains(&plan[s..i]) {
                continue;
            }
            &plan[s..i]
        };
        // The field name.
        let mut end = i + 1;
        while end < bytes.len() && ident(bytes[end]) {
            end += 1;
        }
        if end == i + 1 {
            continue;
        }
        out.insert(format!("{table}.{}", &plan[i + 1..end]));
    }
    out
}

/// Whether the plan cites a field — the dotted `table.key` form always counts; the bare key counts
/// only when it carries an underscore.
///
/// ⚠️ The underscore condition is not fussiness, it closes a measured false-negative class. A bare
/// `plan.contains(key)` marks a field cited whenever its key happens to be an ordinary English
/// word, and these files carried several: `[k1].rule`, `[decidability].sessions`, `[claim].mode`,
/// `[quality].arbiter`. The word "rule" appears dozens of times in the plan's prose, so `[k1].rule`
/// — a field whose value had already been reduced to the string
/// `"SUPERSEDED_BY_k1_decision_rule_below"` — was scored as consumed by a document that never named
/// it. The gate reported a clean bill for the one field most obviously dead.
///
/// The residual limitation, stated rather than left to be discovered: an underscored key that is
/// mentioned in prose *about* the field without being a citation of it still counts as cited. This
/// class errs toward under-reporting, so a zero here is "no field is provably unread", never "every
/// field is provably read".
fn is_cited(plan: &str, key: &str, dotted: &str) -> bool {
    // BOTH spellings of the dotted form. The plan writes `[census].decision_resolution` — with the
    // brackets — so a check for the bare `census.decision_resolution` matches nothing, and the
    // bracketed form is the only one an underscore-free key can ever satisfy. The sensitivity
    // control below is what surfaced this: the repaired rule passed its negative half and failed
    // its positive one.
    if plan.contains(dotted) {
        return true;
    }
    if let Some((table, field)) = dotted.split_once('.')
        && plan.contains(&format!("[{table}].{field}"))
    {
        return true;
    }
    key.contains('_') && plan.contains(key)
}

/// The three violation sets, plus the counts the non-vacuity assertions read.
#[derive(Debug, Default)]
struct Report {
    dangling_citations: BTreeSet<String>,
    unread_rules: BTreeSet<String>,
    orphan_fields: BTreeSet<String>,
    unresolved_gating: BTreeSet<String>,
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
    let citable: BTreeSet<String> = CITABLE_TABLES.iter().map(|t| (*t).to_string()).collect();
    for cited in cited_dotted_fields(plan, &citable) {
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
            if !is_cited(plan, &field.key, &dotted) {
                report.orphan_fields.insert(tagged);
            }
        }
    }
    report.unresolved_gating = unresolved_gating_paths(&parsed, &defined);
    report
}

/// Class 4 — every `table.field` path inside a `[gating]` row must resolve to a field the frozen
/// files define.
///
/// ⚠️ **Added at Rev 10 because the `[gating]` rows were resolved by NOTHING.** The plan states that
/// every row "is now a list of paths", meaning *resolvable* ones, and that claim had no mechanical
/// backing: mutating a row to `["corpus.arrangment"]` — one letter — fired none of the three gates.
/// Class 1 scans the plan, not the TOML values; class 2 needs a `_rule` suffix; class 3 tests the
/// KEY and never the VALUE; and the freeze hashes the other file. The rows are the mechanism by
/// which an unanswered owner VALUES call blocks a rung, so a typo in one silently unblocks it.
fn unresolved_gating_paths(parsed: &[(&str, Vec<Field>)], defined: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, fields) in parsed {
        for f in fields {
            if f.table != "gating" || !f.key.ends_with("_blocked_by") {
                continue;
            }
            for raw in f.value.split(',') {
                let path = raw.trim().trim_matches(|c| c == '[' || c == ']' || c == '"' || c == ' ');
                if path.is_empty() || !path.contains('.') {
                    continue;
                }
                if !defined.contains(path) {
                    out.insert(format!("{}: {path}", f.key));
                }
            }
        }
    }
    out
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
    assert!(
        report.unresolved_gating.is_empty(),
        "a [gating] row names a field no frozen file defines — the row is what blocks a rung, so an \n         unresolvable path in it silently unblocks that rung: {:?}",
        report.unresolved_gating
    );

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

    // CITABLE_TABLES is hand-maintained on purpose (see its doc), so it is pinned against the files
    // in BOTH directions — otherwise it rots into the very thing it exists to avoid.
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let defined_tables: BTreeSet<String> = [thresholds.as_str(), claim.as_str()]
        .iter()
        .flat_map(|t| parse_fields(t))
        .map(|f| f.table)
        .filter(|t| !t.is_empty())
        .collect();
    let listed: BTreeSet<String> = CITABLE_TABLES.iter().map(|t| (*t).to_string()).collect();
    let unlisted: Vec<&String> = defined_tables.difference(&listed).collect();
    let stale: Vec<&String> = listed.difference(&defined_tables).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "CITABLE_TABLES has drifted from the frozen files.\n  \
         defined but NOT listed (their bare citations would be invisible): {unlisted:?}\n  \
         listed but NO LONGER defined (strike them in the same commit as the removal): {stale:?}"
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

/// Sensitivity control for the Rev 9 repair, and the one that matters most now: **deleting a
/// rung-blocking field must be detectable.**
///
/// This reproduces the exact degenerate substitution an adversarial review of Rev 8 constructed
/// against the previous version of this file. `k1_outcome.undecided_disposition` blocks R1 — it is
/// the sentinel that stops an UNDECIDED K1 from being walked past — and the plan cites it
/// **without brackets**. Under the bracket-only scanner: deleting the table removed it from
/// `defined`, but it had never entered the citation set, so no dangling citation appeared; class 3
/// iterates fields that exist, so a deleted field cannot be an orphan; and `fields_parsed` stayed
/// above its floor. **Green run, with §9's outcome table and `[gating].r1_blocked_by` both naming
/// a field that no longer existed.**
///
/// The deletion is performed on an in-memory copy of the real claim file, so this control tracks
/// the live document rather than a fixture that can drift away from it.
#[test]
fn the_sweep_reports_the_deletion_of_a_rung_blocking_field() {
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    assert!(
        plan.contains("k1_outcome.undecided_disposition"),
        "invariant: the plan must cite the R1-blocking field for this control to be meaningful"
    );
    assert!(
        !plan.contains("[k1_outcome].undecided_disposition"),
        "invariant: the citation must be the UNBRACKETED spelling — that is the spelling the \
         bracket-only scanner was blind to, and the reason this control exists. If the plan is \
         reformatted to the bracketed form, re-derive this test against another unbracketed \
         citation rather than deleting it."
    );

    let clean = sweep(&thresholds, &claim, &plan);
    assert!(
        clean.dangling_citations.is_empty(),
        "invariant: the live documents have no dangling citation, so the one appearing below is \
         the deletion's doing. got={:?}",
        clean.dangling_citations
    );

    // The deletion: drop the assignment line, exactly as removing the table would.
    let deleted: String = claim
        .split_inclusive('\n')
        .filter(|l| !l.trim_start().starts_with("undecided_disposition"))
        .collect();
    assert_ne!(deleted, claim, "invariant: the deletion must change the file");

    let dirty = sweep(&thresholds, &deleted, &plan);
    assert!(
        dirty
            .dangling_citations
            .contains("k1_outcome.undecided_disposition"),
        "RED: deleting the field that blocks R1 was NOT reported. The sentinel that stops an \
         UNDECIDED K1 from being walked past can be removed with this gate still green — which is \
         the state Rev 8 shipped in. dangling={:?}",
        dirty.dangling_citations
    );
}

/// Sensitivity control for class 4 — the one that tests a VALUE rather than a key.
///
/// The mutation is a single transposed letter in a `[gating]` payload, which is what the defect
/// looks like in the wild. Before this class existed it fired nothing: class 1 scans the plan, not
/// the TOML values; class 2 needs a `_rule` suffix; class 3 tests the key and never the value; and
/// the freeze hashes the other file. The row would still have *looked* like a list of paths.
#[test]
fn the_sweep_reports_an_unresolvable_gating_payload() {
    let thresholds = read("docs/VG-CAMPAIGN-THRESHOLDS.toml");
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    assert!(
        claim.contains("corpus.arrangement"),
        "invariant: a [gating] row must name `corpus.arrangement` for this control to be meaningful"
    );
    assert!(
        sweep(&thresholds, &claim, &plan).unresolved_gating.is_empty(),
        "invariant: the live rows all resolve, so the failure below is the mutation's doing"
    );

    // ⚠️ Targets the ROW, not the first textual match. `replacen` on the bare field name hits an
    // earlier mention in a comment and leaves the row untouched — this control caught that on its
    // first run, and it is the third time in this campaign that a sensitivity control mutated prose
    // instead of the thing it names.
    let typo = claim.replacen("[\"corpus.arrangement\"]", "[\"corpus.arrangment\"]", 1);
    assert_ne!(typo, claim, "invariant: the mutation must change the file");
    assert!(
        !sweep(&thresholds, &typo, &plan).unresolved_gating.is_empty(),
        "RED: a [gating] row naming a field that does not exist was NOT reported. The rows are what          block a rung on an unanswered owner call, so an unresolvable path in one silently unblocks          that rung — and no other class looks at a value."
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
/// the Rev 7 review found in Rev 7's own edit — a rule definition orphaned *inside* the frozen
/// file, where cross-file discipline cannot see it.
///
/// Driven from a synthetic pair rather than from live content, deliberately. The first version of
/// this control stood on `[absolute_mode]`'s real `absolute_floor_ms_rule` / `absolute_gate_rule`
/// pair, and Rev 8 removed that whole table — the control's own invariant assertion caught it and
/// failed, which is the behaviour it was written to have. But a control that stops compiling
/// against the document it guards every time the document is restructured is a control that will
/// eventually be deleted rather than re-derived. The scanner's behaviour is a property of the
/// scanner; testing it needs no live table.
#[test]
fn the_sweep_reports_a_rule_whose_only_consumer_stops_naming_it() {
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    // A definition and its only consumer, in the shape `[absolute_mode]` actually used: a
    // `<sym>_rule` key defining `<sym>`, and a second rule whose value names `<sym>`.
    let bound = "[synthetic]\n\
                 control_floor_rule = \"spread * median\"\n\
                 control_gate_rule = \"claim < median AND control_floor < distance\"\n";
    let orphaned = "[synthetic]\n\
                    control_floor_rule = \"spread * median\"\n\
                    control_gate_rule = \"claim < median AND renamed_floor < distance\"\n";

    let before = sweep(bound, &claim, &plan);
    assert!(
        !before
            .unread_rules
            .contains("thresholds:synthetic.control_floor_rule"),
        "invariant: while `control_gate_rule` names `control_floor`, the definition is READ — that \
         is what makes the rename below a change. unread={:?}",
        before.unread_rules
    );

    let after = sweep(orphaned, &claim, &plan);
    assert!(
        after
            .unread_rules
            .contains("thresholds:synthetic.control_floor_rule"),
        "RED: a rule definition left with no consumer inside the same file was NOT reported. Class \
         2 is blind, and this gate would not have caught `absolute_effective_floor_rule` — the \
         defect it was written for. unread={:?}",
        after.unread_rules
    );
}

/// Sensitivity control for the citation rule itself, added at Rev 8 because the first version of
/// this gate had a measured false negative there.
///
/// A bare `plan.contains(key)` scores a field as cited whenever its key is an ordinary English
/// word. `[k1].rule` was the specimen: its value had already been reduced to the string
/// `"SUPERSEDED_BY_k1_decision_rule_below"`, and the gate reported it consumed because the word
/// "rule" appears throughout the plan's prose. This asserts the repaired rule both ways — an
/// underscore-free key is NOT satisfied by a prose word, and IS satisfied by a real dotted
/// citation.
#[test]
fn a_bare_english_word_key_is_not_counted_as_a_citation() {
    let claim = read("docs/VG-CAMPAIGN-CLAIM.toml");
    let plan = read("docs/MESHLET-VIRTUAL-GEOMETRY-PLAN.md");

    assert!(
        plan.contains("rule"),
        "invariant: the plan's prose must contain the bare word `rule` for this control to be \
         meaningful"
    );

    let word_key = "[synthetic]\nrule = \"superseded\"\n";
    assert!(
        sweep(word_key, &claim, &plan)
            .orphan_fields
            .contains("thresholds:synthetic.rule"),
        "RED: a key whose name is an ordinary English word was scored as cited by prose. The \
         orphan class is blind to exactly the field most likely to be dead."
    );

    // And the dotted form still counts, so the repair did not simply make underscore-free keys
    // uncitable — which would trade a false negative for a false positive and be no better.
    let cited_plan = format!("{plan}\n\nThe gate reads `[synthetic].rule` from the frozen file.\n");
    assert!(
        !sweep(word_key, &claim, &cited_plan)
            .orphan_fields
            .contains("thresholds:synthetic.rule"),
        "RED: a field cited in the dotted `[table].field` form was still reported as an orphan — \
         the citation rule has become unsatisfiable for underscore-free keys"
    );
}
