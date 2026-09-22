//! The gate over `docs/WORKFLOW-COST-*.md` — the five documents that describe why landing two rungs
//! cost **27 adversarial passes** (13 on A1, 14 on EG2; 30 workflows including the three-pass
//! plan-split refactor, and 31 runs, because EG2 pass 11 ran twice). An earlier header read
//! "26 adversarial passes", which matched no count in the set.
//!
//! Three checks, in ascending order of what they are worth:
//!
//! 1. [`heading_slugs_are_unique_within_each_file`] — re-slugs every heading and fails on a
//!    collision, because two headings with one slug make every `#anchor` to either of them silently
//!    ambiguous.
//! 2. [`every_intra_set_citation_resolves`] — resolves every markdown link between the five
//!    documents, every `#anchor` into them, every **bare same-file** `](#anchor)` link, and every
//!    `` `path:line` `` coordinate whose file lives under one of the six declared roots. *(The
//!    bare-anchor branch is new this round: the check spelled the target document, so a link that
//!    omitted it — six of which this round added — was resolved by nothing.)*
//! 3. [`every_provenance_row_reproduces`] — **the one that matters.** It parses the ```` ```provenance ````
//!    block in `WORKFLOW-COST-EVIDENCE.md` and re-runs each row's command, asserting a four-link
//!    chain from a sentence in the prose to the state of the tree. This is the mechanism that turns a
//!    wrong figure from prose into a red; the first two checks only keep coordinates honest.
//!
//! # This instrument does not satisfy the class rule it gates, and that is stated, not hidden
//!
//! `WORKFLOW-COST-PROCEDURE.md`'s class rule is *"a behaviour-preserving refactor of production
//! source must leave the instrument green"*. Check 2 fails it: inserting a line above
//! line 623 of `migration_helpers.rs` moves a coordinate this file pins, and the gate reds over a
//! change
//! that altered no behaviour. It is admitted anyway, under a stated exception and a measured one —
//! check 3's yield against *pre-existing* wrong figures is six, found while this gate was being
//! written, where `internal_docs_anchors.rs`'s measured yield against pre-existing rot is zero
//! (`WORKFLOW-COST-LIMITS.md` L4). If check 2 ever costs a pass to repair, delete check 2 and keep
//! check 3; that is the trade this document set argues for everywhere else.
//!
//! # What this gate does NOT check, at its own site
//!
//! * **Whether a row's command measures the thing its sentence names.** The chain binds prose →
//!   command → tree, and has no link for meaning. Rows P05, P06, P07 and P26 re-ran
//!   `grep -c "await agent("` — a pattern blind to agents launched through `parallel([...].map(…))`
//!   thunks — and were green for as long as the prose it certified was wrong by 13 runs. They were
//!   then re-pinned to a better command over the *same workflow sources* and were green for another
//!   round, because a source file cannot say how many agents ran. This is the largest hole in the
//!   instrument and no addition to the chain closes it.
//! * **Whether a row's carrier is independent of the figure's origin.** A row that re-reads the
//!   artifact the sentence was derived from is one carrier with two names: a misreading is made
//!   identically on both sides, so the row cannot disagree. That is the mechanism by which the two
//!   failures above survived. Rows P31-P37 broke it halfway — they read the harness's
//!   `journal.jsonl`, written as the agents ran and by nothing that wrote the prose, so the figure
//!   no longer shares a carrier with the *document*. Both sides of those rows are still the same
//!   single record, though, so they cannot catch a misreading of the journal itself.
//!   **Rows P38-P41 are the first whose two sides are independent of each other**: they compute the
//!   same quantity from the per-run `workflows/wf_<id>.json` and from the per-event
//!   `journal.jsonl`, written by different parts of the harness at different times, and fail unless
//!   the two agree. The chain has no link asking for any of this; it is a property of how a row is
//!   chosen, and the reason the choice is recorded here.
//! * **Whether the two sides of such a row cover the same population.** They must, and nothing
//!   checks it. Comparing the journal's returned-agent count against the summed `agentCount`
//!   *appears* to agree at 284 when the journal side is taken over all 59 journal directories and
//!   the `agentCount` side over the 56 runs that have a run record. It is an artifact of the
//!   mismatched populations and it dissolves at 292-vs-284 once both are taken over the same 56.
//!   Rows P38-P41 name their runs explicitly on both sides for exactly this reason — a
//!   cross-check between two carriers is worth nothing until the population is pinned on both.
//! * **Coordinates written in this file's own spelling.** Check 2 only sees a backticked
//!   `path:LINE` token; the section below explains why these comments deliberately never write one.
//!   The two spellings are disjoint, so **this gate structurally cannot check a single coordinate
//!   it writes about itself** — the exemption is not a policy, it is a consequence of the escape,
//!   and it is stated here rather than left for the next pass to find.
//! * **Agents neither harness record saw.** A `journal.jsonl` records the agents its *workflow*
//!   launched, and a `wf_<id>.json` counts the same population. An agent the orchestrator started
//!   directly through its own Agent tool, outside any workflow, appears in **neither**, so rows
//!   P31-P43 are a census of workflow-launched agents and **not** of the campaign's agents. This
//!   applies with full force to the cross-record rows: two records agreeing is evidence that they
//!   read the same population correctly, and no evidence at all that the population is complete.
//!   Every "agent run" figure in the five documents has always meant the former; the two are not
//!   known to coincide, and nothing here can measure the difference.
//! * **Prose that names no figure.** Most of the five documents is argument. Nothing here reads it.
//! * **Coordinates pinned to a revision** (lines 38-42 of `sprite.rs` **at `e7a16fd9`**) and
//!   coordinates pinned to a working tree that no commit holds (the `ZzIter` probe). Both are on
//!   [`CITATION_EXEMPTIONS`] with a reason, and the list is asserted exact — an unused entry reds
//!   just as a new unlisted miss does.
//! * **Coverage — most figures in the set are unbound.** The 43 rows carry 37 distinct `ctx`
//!   fragments, i.e. they bind 37 lines. Outside the provenance block the five documents have
//!   **464 lines carrying a number of two or more digits** (801 carrying any digit at all), so the
//!   bound fraction is **8.0 %** on the tighter denominator and **4.6 %** on the looser one.
//!   **Both denominators move on every edit to any of the five documents.** They read 364 and 676
//!   two rounds ago and 434 and 762 last round; on both occasions the round's own edits moved them
//!   and they were not re-derived, which is the same defect class this file gates: a figure about
//!   the edit set you are inside. They are re-measured
//!   at the end of each round by running, over the five documents with the ```` ```provenance ````
//!   block stripped, a count of lines matching `[0-9]{2}` and of lines matching `[0-9]`. No
//!   provenance row can pin them, because a row's `doc` must be one of the five documents and these
//!   figures live here.
//!   An independent pass mutation-tested the 26-row set and got a green out of a wrong `753`, wrong
//!   yield-curve subtotals, an absurd 4-agent duration range and a wrong-but-in-range line number.
//! * **The yield curve was NOT covered by P28/P29/P30, and an earlier version of this comment said
//!   it was.** Those three rows pin the *row counts of the two findings tables* (13 class-1 rows,
//!   14 class-2 rows) and EG2's production-code total. A row count catches an added or deleted table
//!   row; it cannot catch a wrong number inside the curve, which is where the stopping point
//!   actually rests. **Row P36 now binds the curve's own numerator** — the 67 launches over the
//!   zero-yield passes — against the journal, and P31-P35 bind its denominators. What is still
//!   unbound in that section: the per-pass finding counts, the pass-by-pass zero-yield *membership*
//!   (which passes are in the set is a judgement, not a count), and every wall-clock figure.
//!
//! # Off this machine this target is a hard RED, not a skip
//!
//! Five of the six roots are absolute paths outside this repository, and [`EXECUTED_ROWS`] is a
//! literal the run must hit exactly. On a checkout without `D:/wt/ui`, without the workflow-script
//! directory, without that session's scratchpad, or without that session's
//! `subagents/workflows/` journal directory or its `workflows/` run records, this target **fails** —
//! it does not skip and it does not pass vacuously. The `journal` and `runmeta` roots are the newest
//! and the most perishable: both are written by the harness under the session path, so they
//! disappear when the session's working files are cleaned up even on this machine, and rows P31-P43
//! go red when they do. That is deliberate (a check that quietly skips a population is this
//! campaign's signature defect) and it is a real cost: in a repository whose `CLAUDE.md` leans on a
//! green `cargo test --workspace --all-targets --no-fail-fast`, this target reds for everyone else.
//! If that ever matters more than the guarantee, delete the whole file — do **not** lower
//! [`EXECUTED_ROWS`] to match, which converts the guarantee into the vacuum it was written against.
//!
//! # Its comments deliberately avoid the `<name>.rs:<line>` spelling
//!
//! `tests/internal_docs_anchors.rs` binds that spelling wherever it appears in a Rust source and
//! re-derives its own totals from the count. Writing it here moved that census's `242` to `244` and
//! pushed its dead-path cap from 5 to 6 — measured: this target was green with this file absent and
//! red with it present, both runs pasted in the landing report. That is exactly the class-3 defect
//! this document set is about, produced by the gate written against it. The coordinates below are
//! therefore spelled "line N of `file.rs`" in prose, and appear in the `<name>.rs:<line>` form only
//! inside string literals, which that census skips as data.
//! * **External figures.** `SQLite runs at ~590 test lines per library line` is not derivable here
//!   and has no provenance row; `WORKFLOW-COST-EVIDENCE.md`'s omission block says so.
//! * **Wall clock.** Every duration in the set comes from file mtimes, which a fresh checkout does
//!   not preserve. No row measures one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The five documents, in the order their index lists them.
const DOCS: &[&str] = &[
    "WORKFLOW-COST-PLAN.md",
    "WORKFLOW-COST-EVIDENCE.md",
    "WORKFLOW-COST-PROCEDURE.md",
    "WORKFLOW-COST-BRIEFS.md",
    "WORKFLOW-COST-LIMITS.md",
];

/// Roots a `cwd:` field or a `path:line` citation may resolve against.
///
/// Four of the five are absolute machine paths outside this repository. A machine without them
/// cannot run the rows that use them — which is why [`EXECUTED_ROWS`] exists.
///
/// `journal` and `runmeta` are the harness's own records, and they are the only roots in this list
/// that no human wrote. Rows P31-P37 read the journal; **rows P38-P41 read both and compare them**,
/// which is the only place in this gate where the two sides of a check were written by different
/// parts of the machine; P42 and P43 read the run records alone. See the section on independent
/// carriers in the module header.
///
/// `runmeta` is the parent of `scripts`, so `basename_index` walks the workflow scripts twice. That
/// is harmless — the index maps a basename to every path that has it and resolves if *any* is long
/// enough, and the duplicates are the same files.
fn roots() -> [(&'static str, PathBuf); 6] {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| {
        std::env::var("HOME").unwrap_or_default()
    }));
    let session = "3a949a0f-63dd-4304-b51e-cf1897780494";
    [
        ("reflect", repo),
        ("ui", PathBuf::from("D:/wt/ui")),
        (
            "scripts",
            home.join(format!(
                ".claude/projects/D--claude-BoykoEngine/{session}/workflows/scripts"
            )),
        ),
        (
            "scratch",
            PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join(format!(
                "Temp/claude/D--claude-BoykoEngine/{session}/scratchpad"
            )),
        ),
        (
            "journal",
            home.join(format!(
                ".claude/projects/D--claude-BoykoEngine/{session}/subagents/workflows"
            )),
        ),
        (
            "runmeta",
            home.join(format!(
                ".claude/projects/D--claude-BoykoEngine/{session}/workflows"
            )),
        ),
    ]
}

/// Every provenance row this machine is expected to execute.
///
/// **This literal is the floor that makes a skipped population impossible.** A row whose root is
/// absent, or whose shell is missing, does not quietly drop out: the executed count falls below this
/// number and the test reds naming the rows it could not run. Raise it when rows are added.
///
/// It is also what makes this target a hard red rather than a skip on a machine without the three
/// external roots — see the module header.
const EXECUTED_ROWS: usize = 43;

/// The reason all seven agent-transcript coordinates in [`CITATION_EXEMPTIONS`] carry.
///
/// They name documents that lived only in the lane session's scratchpad, under the SYSTEM TEMP
/// dir: `%LOCALAPPDATA%/Temp/claude/D--claude-BoykoEngine/<session>/scratchpad`. That directory is
/// gone — MEASURED 2026-09-22 on this box: the session's `workflows/` and `subagents/` siblings
/// survive and the `scratchpad` child does not — so the coordinates resolve against no tree and
/// against no commit, here or on any other machine. This is the `ZzIter` entry's class one step
/// further along: that one was never committed; these were never committed AND their carrier has
/// since been deleted. Exempting them RECORDS that; it does not excuse it.
///
/// ONE row of the provenance table has the same cause and is deliberately NOT resolved with it:
/// P27 runs `head -1 a1r_refute.md` in that same directory, so it cannot run at all, and
/// [`EXECUTED_ROWS`] states in its own doc comment that the floor must never be lowered to match a
/// green. Choosing between lowering it, deleting the row and leaving the target red is a ruling for
/// the owner rather than a repair, so it is left red and reported.
const SCRATCH_IS_GONE: &str =
    "an agent transcript that lived only in the lane session's scratchpad under the system temp \
     dir; that directory no longer exists (measured 2026-09-22), so the coordinate resolves \
     against no tree and no commit anywhere";

/// `file:line` coordinates that deliberately do not resolve against the working tree, each with the
/// reason. Asserted **exact**: an entry that stops being needed reds as loudly as a new miss.
const CITATION_EXEMPTIONS: &[(&str, &str)] = &[
    (
        "sprite.rs:38-42",
        "pinned to `e7a16fd9`, the tree the finding was made against; the sentence now sits at :61 \
         and is checked by provenance row P15",
    ),
    (
        "animation.rs:807-822",
        "a `ZzIter` probe that lived only in pass 11's uncommitted working tree; it resolves \
         against no commit, and EVIDENCE says so at its own site",
    ),
    ("a1r4_refute.md:125", SCRATCH_IS_GONE),
    ("a1_attack.md:13", SCRATCH_IS_GONE),
    ("eg2_refute.md:115", SCRATCH_IS_GONE),
    ("a1r6_landCode.md:7", SCRATCH_IS_GONE),
    ("a1r3_ruling.md:21", SCRATCH_IS_GONE),
    ("split_refute.md:117", SCRATCH_IS_GONE),
    ("split_rerun.md:33", SCRATCH_IS_GONE),
];

/// Whitespace-insensitive containment.
///
/// Prose in these documents is hard-wrapped at ~100 columns, so a quoted fragment straddles a
/// newline as often as not. Both sides collapse every run of whitespace to one space before the
/// comparison; without this the `ctx` link would only be checkable on fragments that happen not to
/// wrap, which is a population the gate would be silently choosing.
fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// basename → every file with that name under the declared roots.
///
/// The set's `path:line` citations are written as bare basenames (line 623 of the kernel's
/// `migration_helpers.rs`, for one), so
/// resolution is by name, not by path. A basename with several matches resolves if **any** match is
/// long enough — deliberately weak, and stated here rather than in a claim about the gate's reach.
fn basename_index() -> BTreeMap<String, Vec<PathBuf>> {
    fn walk(dir: &Path, depth: usize, out: &mut BTreeMap<String, Vec<PathBuf>>) {
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !matches!(p.file_name().and_then(|s| s.to_str()), Some("target" | ".git")) {
                    walk(&p, depth - 1, out);
                }
            } else if matches!(p.extension().and_then(|s| s.to_str()), Some("rs" | "md" | "js"))
                && let Some(n) = p.file_name().and_then(|s| s.to_str())
            {
                out.entry(n.to_string()).or_default().push(p);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (_, r) in roots() {
        walk(&r, 12, &mut out);
    }
    out
}

fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs")
}

/// Reads a document, NORMALISING CRLF to LF.
///
/// Not a nicety: every parser below is written against LF — `provenance_rows` splits on the
/// fence plus a newline, and the citation scan works line by line — while a
/// `core.autocrlf=true` checkout, which is what this repository gets on Windows, follows that
/// fence with CR LF. The split therefore matched nothing and the gate refused with
/// *"WORKFLOW-COST-EVIDENCE.md has no ```provenance block"*, a sentence about the DOCUMENT for
/// a fact about the CHECKOUT. MEASURED 2026-09-22 at byte offset 64536 of that file, where the
/// fence really is. A gate whose answer depends on how git materialised the working tree is
/// deciding on a checkout artifact, and the normalisation therefore happens once, here, rather
/// than at each parse site.
fn read(p: &Path) -> String {
    std::fs::read_to_string(p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .replace("\r\n", "\n")
}

/// GitHub's heading-anchor slug: lowercase, drop everything that is not alphanumeric, space,
/// hyphen or underscore, then spaces to hyphens.
fn slug(heading: &str) -> String {
    let mut s = String::with_capacity(heading.len());
    for ch in heading.trim().chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            s.extend(ch.to_lowercase());
        } else if ch == ' ' {
            s.push('-');
        }
    }
    s
}

/// Headings of one document, as `(slug, raw heading text)`, in file order.
fn headings(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let t = line.trim_start();
        if t.starts_with('#') {
            let text = t.trim_start_matches('#').trim();
            if !text.is_empty() {
                out.push((slug(text), text.to_string()));
            }
        }
    }
    out
}

#[test]
fn heading_slugs_are_unique_within_each_file() {
    let mut bad = Vec::new();
    for doc in DOCS {
        let mut seen: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (s, text) in headings(&read(&docs_dir().join(doc))) {
            seen.entry(s).or_default().push(text);
        }
        for (s, texts) in seen {
            if texts.len() > 1 {
                bad.push(format!("{doc}: slug `#{s}` is claimed by {} headings: {texts:?}", texts.len()));
            }
        }
    }
    assert!(bad.is_empty(), "colliding heading anchors:\n  {}", bad.join("\n  "));
}

#[test]
fn every_intra_set_citation_resolves() {
    let dir = docs_dir();
    let anchors: BTreeMap<&str, BTreeSet<String>> = DOCS
        .iter()
        .map(|d| (*d, headings(&read(&dir.join(d))).into_iter().map(|(s, _)| s).collect()))
        .collect();
    let roots = roots();
    let index = basename_index();
    let mut bad = Vec::new();
    let mut hit_exemption: BTreeSet<&str> = BTreeSet::new();

    for doc in DOCS {
        let body = read(&dir.join(doc));
        // (a) markdown links into the set: `](WORKFLOW-COST-X.md#anchor)`.
        for (i, _) in body.match_indices("](WORKFLOW-COST-") {
            let rest = &body[i + 2..];
            let Some(end) = rest.find(')') else { continue };
            let target = &rest[..end];
            let (file, frag) = target.split_once('#').unwrap_or((target, ""));
            match anchors.get(file) {
                None => bad.push(format!("{doc}: link to unknown document `{file}`")),
                Some(set) if !frag.is_empty() && !set.contains(frag) => {
                    bad.push(format!("{doc}: `{file}#{frag}` — no heading in {file} slugs to that"));
                }
                Some(_) => {}
            }
        }
        // (a2) same-file links written bare: `](#anchor)`.
        //
        // Branch (a) only sees links that spell the target document, so every `](#…)` inside a
        // document was unchecked until this round — and this round added six of them. The blind spot
        // was found by hand, which is the argument for checking it mechanically now.
        for (i, _) in body.match_indices("](#") {
            let rest = &body[i + 3..];
            let Some(end) = rest.find(')') else { continue };
            let frag = &rest[..end];
            if !anchors[doc].contains(frag) {
                bad.push(format!("{doc}: `#{frag}` — no heading in {doc} slugs to that"));
            }
        }
        // (b) backticked `path/to/file.ext:LINE[-LINE]` coordinates.
        for tok in body.split('`').skip(1).step_by(2) {
            let Some((path, lines)) = tok.rsplit_once(':') else { continue };
            if !path.ends_with(".rs") && !path.ends_with(".md") && !path.ends_with(".js") {
                continue;
            }
            let first = lines.split('-').next().unwrap_or("");
            if first.is_empty() || !first.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let n: usize = first.parse().unwrap();
            if let Some((key, _)) = CITATION_EXEMPTIONS.iter().find(|(k, _)| tok.ends_with(k)) {
                hit_exemption.insert(key);
                continue;
            }
            let direct = roots
                .iter()
                .map(|(_, r)| r.join(path))
                .find(|p| p.is_file())
                .map(|p| vec![p]);
            let base = path.rsplit('/').next().unwrap_or(path);
            let cands = direct.or_else(|| index.get(base).cloned());
            match cands {
                None => bad.push(format!("{doc}: `{tok}` — no file `{path}` under any declared root")),
                Some(ps) if !ps.iter().any(|p| read(p).lines().count() >= n) => {
                    let longest = ps.iter().map(|p| read(p).lines().count()).max().unwrap_or(0);
                    bad.push(format!(
                        "{doc}: `{tok}` — the longest of {} candidate file(s) named `{base}` has only {longest} lines",
                        ps.len()
                    ));
                }
                Some(_) => {}
            }
        }
    }
    let unused: Vec<_> = CITATION_EXEMPTIONS
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| !hit_exemption.contains(k))
        .collect();
    assert!(
        bad.is_empty() && unused.is_empty(),
        "unresolved citations:\n  {}\nCITATION_EXEMPTIONS entries no citation uses (delete them): {unused:?}",
        if bad.is_empty() { "(none)".into() } else { bad.join("\n  ") }
    );
}

/// One parsed row of the ```` ```provenance ```` block.
struct Row {
    id: String,
    doc: String,
    ctx: String,
    fig: String,
    cwd: String,
    cmd: String,
    out: String,
}

fn provenance_rows(body: &str) -> Vec<Row> {
    let block = body
        .split_once("```provenance\n")
        .expect("WORKFLOW-COST-EVIDENCE.md has no ```provenance block")
        .1
        .split_once("\n```")
        .expect("the ```provenance block is not closed")
        .0;
    let mut rows = Vec::new();
    let mut f: BTreeMap<&str, String> = BTreeMap::new();
    for line in block.lines().chain(std::iter::once("")) {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            if matches!(k, "id" | "doc" | "ctx" | "fig" | "cwd" | "cmd" | "out") && !f.contains_key(k) {
                f.insert(
                    match k {
                        "id" => "id", "doc" => "doc", "ctx" => "ctx", "fig" => "fig",
                        "cwd" => "cwd", "cmd" => "cmd", _ => "out",
                    },
                    v.trim().to_string(),
                );
                continue;
            }
        }
        if line.trim().is_empty() && !f.is_empty() {
            let take = |k: &str| f.get(k).unwrap_or_else(|| panic!("row missing `{k}:`")).clone();
            rows.push(Row {
                id: take("id"), doc: take("doc"), ctx: take("ctx"), fig: take("fig"),
                cwd: take("cwd"), cmd: take("cmd"), out: take("out"),
            });
            f.clear();
        }
    }
    rows
}

#[test]
fn every_provenance_row_reproduces() {
    let dir = docs_dir();
    let evidence = read(&dir.join("WORKFLOW-COST-EVIDENCE.md"));
    let rows = provenance_rows(&evidence);
    let roots = roots();
    let bodies: BTreeMap<&str, String> = DOCS
        .iter()
        .map(|d| {
            // The provenance block is stripped, so a row cannot satisfy its own `ctx` check.
            let b = read(&dir.join(d));
            let stripped = match b.split_once("```provenance\n") {
                Some((head, tail)) => format!("{head}{}", tail.split_once("\n```").map_or("", |x| x.1)),
                None => b,
            };
            (*d, squeeze(&stripped))
        })
        .collect();

    let mut bad = Vec::new();
    let mut executed = 0usize;
    let mut unrunnable = Vec::new();

    for r in &rows {
        // Link 1: the prose fragment is in the document it claims.
        match bodies.get(r.doc.as_str()) {
            None => bad.push(format!("{}: unknown doc `{}`", r.id, r.doc)),
            Some(b) if !b.contains(&squeeze(&r.ctx)) => {
                bad.push(format!("{}: ctx not found verbatim in {} — `{}`", r.id, r.doc, r.ctx));
            }
            Some(_) => {}
        }
        // Link 2: the figure is inside that fragment.
        if !r.ctx.contains(&r.fig) {
            bad.push(format!("{}: fig `{}` is not inside its own ctx", r.id, r.fig));
        }
        // Link 3: the figure is inside the command's declared output.
        if !r.out.contains(&r.fig) {
            bad.push(format!("{}: fig `{}` is not inside out `{}`", r.id, r.fig, r.out));
        }
        // Link 4: the command still prints that output.
        let Some((_, root)) = roots.iter().find(|(n, _)| *n == r.cwd) else {
            bad.push(format!("{}: unknown cwd `{}`", r.id, r.cwd));
            continue;
        };
        if !root.is_dir() {
            unrunnable.push(format!("{} (root `{}` = {} is absent)", r.id, r.cwd, root.display()));
            continue;
        }
        match Command::new("bash").arg("-c").arg(&r.cmd).current_dir(root).output() {
            Err(e) => unrunnable.push(format!("{}: cannot spawn bash: {e}", r.id)),
            Ok(o) => {
                executed += 1;
                let got = String::from_utf8_lossy(&o.stdout).replace('\r', "");
                let got = got.trim();
                if !o.status.success() {
                    bad.push(format!("{}: `{}` exited {:?}", r.id, r.cmd, o.status.code()));
                } else if got != r.out {
                    bad.push(format!(
                        "{}: `{}`\n      in {}\n      expected `{}`\n      got      `{}`",
                        r.id, r.cmd, r.cwd, r.out, got
                    ));
                }
            }
        }
    }

    assert!(
        bad.is_empty(),
        "{} of {} provenance rows do not reproduce:\n  {}",
        bad.len(),
        rows.len(),
        bad.join("\n  ")
    );
    assert_eq!(
        executed, EXECUTED_ROWS,
        "the provenance table ran {executed} rows, not the {EXECUTED_ROWS} this machine must run. \
         Rows it could not run: {unrunnable:?}. This assertion exists because a check that quietly \
         skips a population is this campaign's signature defect — do not lower it to match a green."
    );
}
