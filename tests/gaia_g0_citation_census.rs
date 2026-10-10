//! **Gaia rung G0's gate, mechanised.** Three censuses over the `docs/aether-v2/` +
//! `docs/gaia/` corpus, each reporting the counts it enforced.
//!
//! # Why this file exists
//!
//! G0's gate ([`docs/gaia/CAMPAIGN.md`](../docs/gaia/CAMPAIGN.md) §Rung ladder, row
//! `G0`) is written as prose:
//!
//! > docs exist before any grammar commit **AND** every AIR cross-note on a ratified
//! > line in gaia DECISIONS resolves to a defined AIR item … A file that RESOLVES but
//! > is STALE against the ratified syntax does not satisfy a cross-reference.
//!
//! and [`docs/aether-v2/KERNEL-BACKLOG.md`](../docs/aether-v2/KERNEL-BACKLOG.md)
//! §Id namespace states a second property explicitly as *"a recipe to run, not a
//! result asserted here"*:
//!
//! > every `\b(KE|KM)[0-9]+\b` hit over `docs/` resolves to a row in this file, and
//! > the retired bare `E#`/`M#` form appears nowhere as a citation of it. (Both
//! > halves are recipes to run, not results asserted here; the first was run to
//! > produce the list, the second was not.)
//!
//! A property nobody runs is the repo's standing failure class — a gate that cannot
//! fail is not a gate. This file runs all of it.
//!
//! **No waivers, by construction — and since 2026-08-30 that is a RULING, not just a
//! habit.** Ballot GB-8 asked whether a corpus-wide census may carry per-site waivers
//! and was ruled **no, at all four censuses here, permanently**
//! ([`docs/OPEN-QUESTIONS.md`](../docs/OPEN-QUESTIONS.md) §2026-08-29). Nothing here
//! carries a skip list. Where a property turned out **not to be decidable** as
//! written, it is narrowed to a predicate that *is* decidable and the narrowing is
//! stated in the failure message — never papered over with a per-site exception. A
//! **scope statement** ("this census covers directory X") is not a waiver; a per-site
//! skip list is.
//!
//! ⚠ **The precedent figure this file used to cite is not the tree's.** It read
//! "a per-site waiver clause abdicated 188 of 302 anchors to known rot". No in-tree
//! gate produces those numbers — [`tests/internal_docs_anchors.rs`](internal_docs_anchors.rs),
//! the sibling gate that carries the history, states neither. Run live on 2026-08-30
//! it prints **735 anchors, 116 waived** (ARCHITECTURE 6/0, FEATURE_MAP 222/7,
//! SYSTEMS 330/20, MESHLET-VIRTUAL-GEOMETRY-PLAN **177/89**). The aggregate is 15.8%,
//! not 62% — **and the precedent is stronger for it**: the waiver did not spread, it
//! concentrated entirely in the one document admitted under the allowance, which now
//! waives **50.3%** of its anchors, and a waived anchor keeps *neither* shape *nor*
//! identity. That is the measured ground GB-8's ruling stands on.
//!
//! # Scope, and the widening this file still owes
//!
//! The AIR census runs over `docs/gaia/` **and** `docs/aether-v2/` — G0's own scope.
//! The `KE#`/`KM#` census does run over all of `docs/`, because that is the scope the
//! backlog's own published recipe names.
//!
//! ⚠ **GB-8 has been RULED (2026-08-30) and this file has not yet been changed to
//! match.** The ruling: the AIR census **widens to all of `docs/`, and it lands at
//! G0** — here, in this file, as a one-constant change ([`G0_DIRS`] →
//! `markdown_under("docs")`). It was measured green: the `AIR-##` citations outside
//! the two directories sit in **5** files (`docs/OPEN-QUESTIONS.md`,
//! `docs/ru/OPEN-QUESTIONS.md`, `docs/AETHER-GAIA-REVISION-2026-08-29.md`,
//! `docs/FEATURE_MAP.md`, `docs/AETHER-V1-SURFACE-REVIEW.md`), and **every id cited
//! lies inside the carrier's `AIR-01..AIR-18`** — so the widening costs **zero**
//! remediation. ⚠ The *count* is deliberately not pinned in this comment: it measured
//! 21 before GB-8's ruling was written and 40 after, because the ruling text itself
//! cites `AIR-06` repeatedly. A citation count goes stale on the next edit; the
//! property — every cited id resolves — is what this file enforces, and it is the one
//! worth writing down. The pass that ruled GB-8 deliberately did
//! not make the edit: it decides and records, it does not build a rung. **G0 owes
//! this change**, and until it lands, this paragraph is the record that the scope
//! below is narrower than the ruling.
//!
//! The **link** half of GB-8 is a different deliverable and was ruled onto **Aether
//! R8**, not here: measured 1636 relative markdown targets under `docs/`, **59 dead
//! across 11 files** (44 of them in `docs/AUDIT-2026-05-23.md`), versus **0 dead** of
//! 116 inside the two directories this file already covers.
//!
//! # Home
//!
//! The workspace-root package (`boyko-engine`), for the two reasons
//! `internal_docs_anchors.rs` records: `CARGO_MANIFEST_DIR` **is** the repository
//! root, so no `../..` walking can silently aim the scan at the wrong tree; and that
//! package has zero dependencies, so this gate needs no GPU and no engine build.
//! Hand-rolled scanning for the same reason — there is no regex crate to reach for.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The AI-orientation requirement set — the definition carrier for `AIR-##`.
const AIR_CARRIER: &str = "docs/aether-v2/AI-ORIENTATION.md";

/// The kernel backlog — the definition carrier for `KE#` / `KM#`.
const BACKLOG_CARRIER: &str = "docs/aether-v2/KERNEL-BACKLOG.md";

/// The revision record whose per-file table marks files `ratified-stale`. The stale
/// set is READ from here rather than hand-listed in this file: a hand list goes green
/// over divergence, which is the defect this corpus keeps measuring.
const REVISION_RECORD: &str = "docs/AETHER-GAIA-REVISION-2026-08-29.md";

/// The two directories G0's own gate covers.
const G0_DIRS: [&str; 2] = ["docs/gaia", "docs/aether-v2"];

/// The repository root (`CARGO_MANIFEST_DIR` of the root package).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Reads a corpus file, panicking with the path when it is missing (a moved carrier
/// must be a loud failure, never a silently empty census).
fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("census carrier {} unreadable: {e}", p.display()))
}

/// Every `*.md` under `dir`, relative to the repo root, sorted.
fn markdown_under(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let root = repo_root();
    walk(&root.join(dir), &root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            let rel = p.strip_prefix(root).unwrap_or(&p);
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

// ───────────────────────────── token scanning ──────────────────────────────

/// True when `b` cannot be part of an identifier — the boundary an id must sit
/// between. `-` counts as a boundary character so `AIR-01` is found inside prose,
/// but the *preceding* check below additionally rejects an alphanumeric run before
/// the prefix, so `FAIR-01` never matches `AIR-01`.
fn is_boundary(b: u8) -> bool {
    !(b.is_ascii_alphanumeric() || b == b'_')
}

/// Every `<prefix><digits>` occurrence in `text`, as `(line_number_1_based, id)`.
///
/// The prefix must start at a non-identifier boundary (so `KE1` is not found inside
/// `SKE1`, and `E1` is not found inside `KE1` — the distinction the retired-form
/// half of the backlog recipe turns on), and the digit run must end at one (so
/// `KE1` is not reported for the text `KE13`).
fn occurrences(text: &str, prefix: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let pb = prefix.as_bytes();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        let starts_here = bytes[i..].starts_with(pb)
            && (i == 0 || is_boundary(bytes[i - 1]));
        if !starts_here {
            i += 1;
            continue;
        }
        let mut j = i + pb.len();
        let ds = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j == ds || (j < bytes.len() && !is_boundary(bytes[j])) {
            i += 1;
            continue;
        }
        out.push((line, text[i..j].to_string()));
        i = j;
    }
    out
}

/// The ids `prefix` DEFINES in `text`: the leading cell of a markdown table row.
///
/// A definition row is `| <id> …` or `| **<id>** …` — the two shapes the carriers
/// use (`| AIR-01 | …`, `| KE1 ✅ **LANDED …**`, `| KM1 ✅ **LANDED 2026-08-30** |`).
/// Reading the leading cell only is what keeps a *mention* of `KE13` inside another
/// row's prose from being mistaken for `KE13`'s own definition.
fn definitions(text: &str, prefix: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in text.lines() {
        let line = raw.trim_start();
        let Some(rest) = line.strip_prefix('|') else {
            continue;
        };
        let cell = rest.trim_start().trim_start_matches('*');
        if !cell.starts_with(prefix) {
            continue;
        }
        let digits: String = cell[prefix.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            continue;
        }
        // The id must END the token — `| KE1 …` defines KE1, `| KE13 …` defines KE13.
        let after = &cell[prefix.len() + digits.len()..];
        if after.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        out.insert(format!("{prefix}{digits}"));
    }
    out
}

/// Formats an unresolved-citation report: id → the sites that cite it.
fn render(unresolved: &BTreeMap<String, Vec<String>>) -> String {
    let mut s = String::new();
    for (id, sites) in unresolved {
        s.push_str(&format!("\n  {id} — cited at:"));
        for site in sites {
            s.push_str(&format!("\n      {site}"));
        }
    }
    s
}

// ─────────────────────────── 1. the AIR census ─────────────────────────────

/// **G0's gate, first clause.** Every `AIR-##` cited anywhere in `docs/gaia/` or
/// `docs/aether-v2/` resolves to a defined row in `AI-ORIENTATION.md`.
///
/// The gate the campaign wrote is scoped to "a ratified line in gaia DECISIONS".
/// This runs it over both directories and every line, which is strictly wider and
/// therefore cannot go green where the narrow form would go red. Widening a census
/// is free; narrowing one is how a gate stops guarding.
#[test]
fn air_cross_notes_resolve_to_defined_items() {
    let defined = definitions(&read(AIR_CARRIER), "AIR-");
    assert!(
        defined.len() >= 18,
        "the AIR requirement set parsed as only {} rows — the table shape changed and \
         this census would go vacuously green. Parsed: {:?}",
        defined.len(),
        defined
    );

    let mut files = Vec::new();
    for d in G0_DIRS {
        files.extend(markdown_under(d));
    }

    let mut cited = 0usize;
    let mut unresolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rel in &files {
        let text = read(rel);
        for (line, id) in occurrences(&text, "AIR-") {
            cited += 1;
            if !defined.contains(&id) {
                unresolved.entry(id).or_default().push(format!("{rel}:{line}"));
            }
        }
    }

    println!(
        "AIR census: {} definitions in {AIR_CARRIER}; {cited} citations across {} files in {:?}",
        defined.len(),
        files.len(),
        G0_DIRS
    );
    assert!(
        unresolved.is_empty(),
        "{} AIR citation(s) resolve to no defined item in {AIR_CARRIER}.{}\n\n\
         G0's gate is that every AIR cross-note resolves. Fix the citation or define \
         the item — this census carries NO waiver list, by ballot GB-8's ruling \
         (2026-08-30): on the sibling anchor gate, the one document admitted under a \
         waiver allowance now waives 89 of its 177 anchors (50.3%), and a waived \
         anchor keeps neither shape nor identity.",
        unresolved.values().map(Vec::len).sum::<usize>(),
        render(&unresolved)
    );
}

// ───────────────── 2. the kernel-backlog id census (the unrun recipe) ──────────────

/// **The backlog's own published recipe, first half.** Every `KE#` / `KM#` anywhere
/// under `docs/` resolves to a row in `KERNEL-BACKLOG.md`.
///
/// Runs over all of `docs/` — the scope the recipe itself names — because the
/// `KE`/`KM` prefixes are a minted namespace with no competing carrier, so the wide
/// scan has no false-positive population. (Contrast the retired bare form below,
/// where the wide scan is exactly what does not work.)
#[test]
fn kernel_backlog_ids_resolve_to_backlog_rows() {
    let backlog = read(BACKLOG_CARRIER);
    let mut defined = definitions(&backlog, "KE");
    defined.extend(definitions(&backlog, "KM"));
    assert!(
        defined.len() >= 16,
        "the kernel backlog parsed as only {} rows — the table shape changed and this \
         census would go vacuously green. Parsed: {:?}",
        defined.len(),
        defined
    );

    let files = markdown_under("docs");
    let mut cited = 0usize;
    let mut unresolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rel in &files {
        let text = read(rel);
        for prefix in ["KE", "KM"] {
            for (line, id) in occurrences(&text, prefix) {
                cited += 1;
                if !defined.contains(&id) {
                    unresolved.entry(id).or_default().push(format!("{rel}:{line}"));
                }
            }
        }
    }

    println!(
        "KE/KM census: {} rows in {BACKLOG_CARRIER}; {cited} citations across {} markdown \
         files under docs/",
        defined.len(),
        files.len()
    );
    assert!(
        unresolved.is_empty(),
        "{} KE/KM citation(s) resolve to no row in {BACKLOG_CARRIER}.{}",
        unresolved.values().map(Vec::len).sum::<usize>(),
        render(&unresolved)
    );
}

/// **The backlog's recipe, second half — NARROWED, and the narrowing is the finding.**
///
/// The recipe reads *"the retired bare `E#`/`M#` form appears nowhere as a citation
/// of it"*. Taken literally over `docs/`, that is **not a decidable property**, and
/// this test does not pretend otherwise. Measured while writing it: a bare `E#`/`M#`
/// scan over `docs/**/*.md` returns **1444 hits**, and the overwhelming majority
/// belong to live, unrelated id series that were never renamed —
///
/// * `M1`–`M9`, the machine rulings in `docs/aether-v2/DECISIONS.md`;
/// * `M1`–`M19`, the conformance findings in `docs/gaia/PENDING-SYNTAX-PLAN.md`
///   (a different series again, and `AI-ORIENTATION.md` AIR-07 says so in as many
///   words: *"that file's own `M#` finding series — **not** this corpus's `M1`–`M9`
///   machine rulings"*);
/// * `M1`–`M6`, milestones in `docs/ARCHITECTURE-HYBRID-PERF.md` and several
///   `docs/archive/` plans.
///
/// The backlog's stated exception names exactly **one** of those carriers (a bare
/// `E1`/`E2`/`E3` inside `aether-v2/DECISIONS.md`) and is silent on the other three,
/// the largest of which sits in the same directory. Enforcing the literal form would
/// therefore require ~1400 per-site dispositions — which is the waiver outcome the
/// corpus forbids (GB-8's ruling, 2026-08-30), reached by a different road.
///
/// So the property is narrowed to one that IS decidable, with no waiver list:
/// **a bare `E#`/`M#` on a line that references `KERNEL-BACKLOG.md` is a retired
/// citation of it.** That is the shape a real citation takes ("see KERNEL-BACKLOG.md
/// E11"), it has no false-positive population, and it guards against reintroduction.
/// The carrier file itself is excluded — its own §Id namespace paragraph *quotes* the
/// retired spellings to define the rename, and quoting a retired id in the sentence
/// that retires it is not a citation of it.
#[test]
fn retired_bare_ids_are_not_cited_as_backlog_ids() {
    let files = markdown_under("docs");
    let mut scanned_lines = 0usize;
    let mut offenders: Vec<String> = Vec::new();

    for rel in &files {
        if rel == BACKLOG_CARRIER {
            continue;
        }
        let text = read(rel);
        for (n, line) in text.lines().enumerate() {
            if !line.contains("KERNEL-BACKLOG") {
                continue;
            }
            scanned_lines += 1;
            for prefix in ["E", "M"] {
                for (_, id) in occurrences(line, prefix) {
                    // `occurrences` already refuses a match preceded by an
                    // identifier character, so `KE11` never yields `E11` here.
                    offenders.push(format!("{rel}:{}  bare `{id}` — write `K{id}`\n      {}", n + 1, line.trim()));
                }
            }
        }
    }

    println!(
        "retired-form census (NARROWED — see this test's doc comment): {scanned_lines} \
         backlog-referencing line(s) across {} markdown files under docs/; \
         {} offender(s)",
        files.len(),
        offenders.len()
    );
    assert!(
        offenders.is_empty(),
        "{} line(s) cite the kernel backlog using the RETIRED bare `E#`/`M#` spelling \
         (renamed to `KE#`/`KM#` on 2026-08-29):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

// ──────────────── 3. the staleness clause (G0's restored second clause) ────────────────

/// The machine-readable marker a stale file must carry in its own head.
const STALE_MARKER: &str = "ratified-stale";

/// How many leading lines of a file count as "its own head" — a reader who opens the
/// file must meet the marker without scrolling.
const HEAD_LINES: usize = 12;

/// **G0's gate, second clause.** *"A file that RESOLVES but is STALE against the
/// ratified syntax does not satisfy a cross-reference."*
///
/// A link check alone cannot see this: a stale file resolves perfectly. What makes
/// the clause enforceable is that the staleness has to be **visible at the file the
/// reader lands on**, not only in the index that pointed there. So: every file the
/// revision record's per-file table marks `ratified-stale` must repeat that marking
/// in its own first [`HEAD_LINES`] lines.
///
/// The stale set is PARSED from [`REVISION_RECORD`], never hand-listed here. A hand
/// list is the failure mode this corpus has already measured twice (the backlog's own
/// citer enumeration went stale the moment a sibling added one; the `.ui` printer's
/// hand-listed comparator went green over a 10-of-19 loss).
#[test]
fn files_marked_ratified_stale_say_so_in_their_own_head() {
    let record = read(REVISION_RECORD);

    // Parse the per-file table: rows whose Substance cell carries the marker, whose
    // File cell carries a markdown link target relative to `docs/`.
    let mut stale: BTreeSet<String> = BTreeSet::new();
    for line in record.lines() {
        let l = line.trim_start();
        if !l.starts_with('|') || !l.contains(STALE_MARKER) {
            continue;
        }
        let Some(file_cell) = l.split('|').nth(1) else {
            continue;
        };
        // `[`gaia/LANGUAGE.md`](gaia/LANGUAGE.md)` → the parenthesised target.
        let mut rest = file_cell;
        while let Some(open) = rest.find("](") {
            let after = &rest[open + 2..];
            if let Some(close) = after.find(')') {
                let target = &after[..close];
                if target.ends_with(".md") && !target.starts_with("http") {
                    stale.insert(format!("docs/{}", target.trim_start_matches("./")));
                }
                rest = &after[close..];
            } else {
                break;
            }
        }
    }

    assert!(
        !stale.is_empty(),
        "no `{STALE_MARKER}` row parsed out of {REVISION_RECORD} — the per-file table \
         changed shape and this census would go vacuously green. The clause it enforces \
         is G0's second one, and a vacuous pass is exactly what it exists to catch."
    );

    let mut unmarked: Vec<String> = Vec::new();
    for rel in &stale {
        let text = read(rel);
        let head: String = text.lines().take(HEAD_LINES).collect::<Vec<_>>().join("\n");
        if !head.contains(STALE_MARKER) {
            unmarked.push(rel.clone());
        }
    }

    println!(
        "staleness census: {} file(s) marked `{STALE_MARKER}` by {REVISION_RECORD}: {:?}",
        stale.len(),
        stale
    );
    assert!(
        unmarked.is_empty(),
        "{} file(s) are marked `{STALE_MARKER}` in {REVISION_RECORD} but do NOT say so \
         in their own first {HEAD_LINES} lines: {:?}\n\n\
         G0's gate: a file that RESOLVES but is STALE does not satisfy a cross-reference. \
         A reader who opens the file directly — or an agent that reads it as the Gaia \
         syntax — meets no warning, and the index that knew is a file they never opened. \
         Add the `{STALE_MARKER}` marking to the file's own head.",
        unmarked.len(),
        unmarked
    );
}
