//! Rung L2b — the registry walker and the mechanical checks over it.
//!
//! An **integration** test, deliberately: `cargo test --workspace --lib` does not build `tests/`,
//! and a gate invisible to the command people actually run is a gate that is not run. That blind
//! spot cost this repository four commits once already.
//!
//! # The walker: ONE pass, THREE disjoint streams
//!
//! The predecessor design specified no walker, and its checks then required **opposite** behaviour
//! from the one it did not specify: a scan that includes comments makes the orphan check
//! satisfiable by writing a `# Panics` line, while a scan that excludes them makes the
//! panic-placement check red on those same doc comments. Both cannot hold under one unspecified
//! walker, so the walker is specified here and every check names the stream it consumes.
//!
//! | Stream | Contents | Consumed by |
//! |---|---|---|
//! | **CODE** | source with line comments, block comments, string/char literals and test-only files removed | 3, 3b, 6, 7 |
//! | **LIT** | the contents of string and char literals only | 4 |
//! | **TEXT** | whole unstripped files, plus an explicit documentation directory list | 0, 4 |
//!
//! It is deliberately **not** a Rust parser. Its known failure mode — a brace count confused by a
//! brace inside a string or comment — is the same one `scripts/check_hotpath_exceptions.py`
//! documents and accepts, for the same reason: a parser is a dependency and a maintenance burden
//! for a job that a lexer-shaped scan does correctly on this corpus.
//!
//! # Which checks are ARMED here, and which are not
//!
//! At L2 every row was `Pending`, which made four of the eight checks vacuous; each was named with
//! the rung it would become real at rather than shipped green. **L6 is that rung for the last
//! three**, and arming them found what a first run is for — see checks 2, 5 and 6 below.
//!
//! | # | Check | State |
//! |---|---|---|
//! | 0 | corpus non-empty + pinned sentinel | **ARMED** |
//! | 1 | numbers strictly increasing, table dense | **ARMED** (also a `const` assert in `codes.rs`) |
//! | 2 | `Live` rows have a doc page **with its three sections** | **ARMED** at L4, TIGHTENED at L6 — the L4 form was `is_file()` alone, so an empty page satisfied it |
//! | 3a | `Live` rows have ≥1 identifier use | **ARMED** at L4 |
//! | 3b | `Pending`/`Historical` rows have 0 identifier uses | **ARMED** — all 32 rows |
//! | 3c | `Pending` count == 0 | **ARMED at L8c** — first run required flipping four rows whose named rungs had already shipped |
//! | 4 | every prefixed literal resolves to a row | **ARMED** |
//! | 5 | every `Live` W/E code is named by a test, or is in the ledger | **ARMED at L6** — first run: 8 of 20 rows unnamed |
//! | 6 | a `B` code is never an argument to an emission macro | **ARMED at L6**, re-specified against the tree |
//! | 7 | every `LogTarget` impl resolves to the table | **ARMED** |
//!
//! Naming the vacuous ones was the point. A gate that cannot fail, shipped as one, is this
//! campaign's signature defect; the table said which rung each became real at instead of letting
//! four green checks imply four proofs — and when they armed, three of them reported real debt.

// `BTreeMap`, not `HashMap`: the workspace bans the latter outright, and a gate wants its failure
// message in a deterministic order anyway -- an unordered list of offending codes reads
// differently on every run and makes two failures look like two defects.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use boyko_log::DIAGNOSTICS;
use boyko_log::codes::CodeStatus;

mod walker;

use walker::{doc_files, production_code, repo, rust_files, split_streams, test_only_files};
/// A registered code's identifier, e.g. `B1802`.
fn ident_of(class: u8, number: u16) -> String {
    format!("{}{number:04}", class as char)
}

/// `true` when `hay[at..]` starts with `needle` as a STANDALONE token.
///
/// Never a substring: `B1802` inside `boyko-B1802` must not count, which is the vacuity that
/// sank two earlier designs of the orphan check. After stripping, the prefixed literal does not
/// exist in CODE at all — this is belt and braces for the path form `codes::B1802`.
fn has_token(hay: &str, needle: &str) -> bool {
    let b = hay.as_bytes();
    let n = needle.as_bytes();
    // `-` is NOT a boundary on the left. Without this clause `B1802` inside `boyko-B1802` reads as
    // a standalone token, which is precisely the substring acceptance that sank two earlier
    // designs of the orphan check -- and the walker's own unit test caught it here.
    let boundary = |c: u8| !(c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let at = from + rel;
        let before_ok = at == 0 || boundary(b[at - 1]);
        let after = at + n.len();
        let after_ok = after >= b.len() || boundary(b[after]);
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
    }
    false
}

/// Every `boyko-[BEW]dddd` literal in `text`, as `(class, number)`.
fn prefixed_literals(text: &str) -> Vec<(u8, u16)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = text[from..].find("boyko-") {
        let at = from + rel + 6;
        from = at;
        if at + 5 > b.len() {
            break;
        }
        let class = b[at];
        if !matches!(class, b'B' | b'E' | b'W') {
            continue;
        }
        let digits = &text[at + 1..at + 5];
        if !digits.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // Reject a longer run of digits: `boyko-B12345` is not a code.
        if at + 5 < b.len() && b[at + 5].is_ascii_digit() {
            continue;
        }
        if let Ok(n) = digits.parse::<u16>() {
            out.push((class, n));
        }
    }
    out
}

/// Everything the checks read, gathered in one pass.
struct Corpus {
    /// CODE, concatenated, from production `.rs` files only.
    code: String,
    /// LIT ∪ TEXT.
    literals: Vec<(u8, u16, String)>,
    /// How many files were scanned in total.
    files_scanned: usize,
    /// TEXT, concatenated.
    text: String,
}

fn gather() -> Corpus {
    let root = repo();
    let rs = rust_files(&root);
    assert!(!rs.is_empty(), "the walker resolved a root with no Rust files: {}", root.display());
    let excluded = test_only_files(&root, &rs);

    let mut code = String::new();
    let mut literals = Vec::new();
    let mut text = String::new();
    let mut files_scanned = 0usize;

    for f in &rs {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        files_scanned += 1;
        let s = split_streams(&src);
        for (c, n) in prefixed_literals(&s.lit) {
            literals.push((c, n, f.display().to_string()));
        }
        // `codes.rs` DEFINES every identifier, so including it makes the orphan check trivially
        // satisfied and the premature-emitter check permanently red. Excluded by name, as the
        // check's own specification requires.
        let is_registry = f.file_name().is_some_and(|n| n == "codes.rs");
        if !excluded.contains(f) && !is_registry {
            // `production_code`, not `s.code`: the corpus defines CODE as excluding in-`src`
            // `#[cfg(test)]` regions too, and until L8c nothing implemented that half. It matters
            // to check 3a — an identifier whose ONLY use is inside a test module is not an
            // emitter, and counting it would let a `Live` row be satisfied by its own observer.
            code.push_str(&production_code(&src));
            code.push('\n');
        }
    }

    for f in doc_files(&root) {
        let Ok(src) = std::fs::read_to_string(&f) else { continue };
        files_scanned += 1;
        for (c, n) in prefixed_literals(&src) {
            literals.push((c, n, f.display().to_string()));
        }
        text.push_str(&src);
        text.push('\n');
    }

    Corpus { code, literals, files_scanned, text }
}

// ─────────────────────────────────── the checks ───────────────────────────────────

/// **Check 0** — the corpus is non-empty and a pinned sentinel is found.
///
/// A walker that resolves its root badly scans zero files and reports zero orphans: green, and
/// proving nothing. rustc's own tidy pins a sentinel for exactly this reason. The sentinel is
/// written here in two halves so that this file does not itself carry the prefixed literal of a
/// code — a rule the corpus adopted after a planning document reddened its own gate permanently by
/// naming an unregistered code in full.
#[test]
fn check_0_corpus_is_non_empty_and_the_sentinel_resolves() {
    let c = gather();
    assert!(
        c.files_scanned >= 500,
        "only {} files scanned; the walker has resolved a wrong or truncated root",
        c.files_scanned
    );
    let sentinel = format!("boyko-{}", "W1501");
    assert!(
        c.text.contains(&sentinel) || c.literals.iter().any(|(cl, n, _)| *cl == b'W' && *n == 1501),
        "the pinned sentinel was not found anywhere; the scan is not reaching the corpus"
    );
}

/// **Check 1** — numbers strictly increasing, and the table dense.
///
/// Also a `const` assert in `codes.rs`; restated here because the const proves the ORDER while
/// this proves the addressing scheme the order exists to support.
#[test]
fn check_1_registry_is_ordered_and_dense() {
    let mut prev: Option<u16> = None;
    for (i, row) in DIAGNOSTICS.iter().enumerate() {
        if let Some(p) = prev {
            assert!(p < row.number, "row {i}: {} does not exceed {p}", row.number);
        }
        prev = Some(row.number);
        assert!(
            matches!(row.class, b'B' | b'E' | b'W'),
            "row {i} has class {:?}, which is not one of B/E/W",
            row.class as char
        );
        assert!(!row.summary.is_empty(), "row {i} has an empty summary");
    }
}

/// **Check 3b** — no premature emitters.
///
/// Every `Pending` or `Historical` row's identifier must appear **zero** times in CODE. This is
/// the check that lets a `Pending` row exist without rotting: the day it acquires an emitter it
/// reds, which forces the row to flip to `Live` in the same commit as the emitter.
#[test]
fn check_3b_no_pending_code_has_an_emitter_yet() {
    let c = gather();
    let mut offenders = Vec::new();
    for row in DIAGNOSTICS {
        if !row.status.forbids_emitter() {
            continue;
        }
        let id = ident_of(row.class, row.number);
        if has_token(&c.code, &id) {
            offenders.push(id);
        }
    }
    assert!(
        offenders.is_empty(),
        "these rows are Pending/Historical but their identifiers appear in CODE: {offenders:?}. \
         Flip the row to Live in the same commit as the emitter -- and give it a doc page, which \
         check 2 will then require."
    );
}

/// **Check 2** — every `Live` row has a `docs/diagnostics/<CODE>.md` page **with its three
/// sections**.
///
/// ARMED AT L4, by the mechanism L2b left for exactly this moment: a test asserted that no row was
/// `Live`, so that the first flip would red and force this check to be written. `W0103` flipped and
/// it did. The vacuity test is deleted rather than kept — a precondition assertion and the check it
/// stood in for cannot both be true.
///
/// **TIGHTENED AT L6 to the form the corpus always specified** — "exists, non-empty, has
/// `## What happened` / `## Why` / `## How to fix`". What shipped at L4 was `is_file()` alone, so
/// an empty file satisfied it, and this rung was the first to add pages in bulk (ten of them) —
/// the moment at which a page check that only counts files stops being worth having. MEASURED on
/// arming: one existing page, `W9212.md`, had no `## Why` section at all; its argument lived under
/// `## Refused, not clamped`, which is a fine subtitle and not a section a reader can look for.
///
/// The match is a **prefix**, not an equality: fourteen of the profiling pages write
/// `## Why it fires once`, `## Why it is a warning and not an error`, and so on. Requiring the
/// bare heading would have forced fourteen pages to say *less*.
///
/// `Pending` and `Historical` rows are out of scope on purpose. A page for a row with no emitter
/// describes a message nobody has written, which is how three rows of this registry's first draft
/// came to disagree with what the engine prints.
#[test]
fn check_3c_no_row_is_still_pending() {
    // ARMED AT L8c, and it is the rung's closing claim: `Pending` is a PROMISE -- "this code is
    // reserved for a named rung and has no emitter yet" -- and a promise with no expiry is a
    // reservation. `Historical` is excluded by design (it promises no emitter, ever).
    //
    // It could not be armed until L8c because four rows were still `Pending`, and every one of
    // them named a profiling rung that had SHIPPED: `W9202`/`W9217` said "profiling 5",
    // `W9205`/`W9206` said "profiling 8". All four conditions were measured present in the tree
    // and silent -- `alloc_pair` returning `None`, a teardown that never flushed, a window's lost
    // pairs, a refused contrast. The rows were not waiting for work; the work had shipped without
    // them.
    //
    // What this check buys from here on is that the state cannot recur: a row reserved for a rung
    // reds the moment that rung is called done, because "done" and "a row still says Pending" have
    // stopped being compatible.
    let pending: Vec<String> = DIAGNOSTICS
        .iter()
        .filter_map(|r| match r.status {
            CodeStatus::Pending(rung) => {
                Some(format!("{}{} (reserved for {rung})", r.class as char, r.number))
            }
            _ => None,
        })
        .collect();
    assert!(
        pending.is_empty(),
        "these rows are still Pending: {pending:?}. A Pending row is a reservation with a named          rung; if that rung has landed, the row owes an emitter, a doc page and an observing test,          and if it has not, the rung owes this row. Neither state survives a rung being called          done -- which is exactly what L8c found when it armed this check and all four remaining          rows named rungs that had shipped."
    );
}

#[test]
fn check_2_every_live_row_has_a_doc_page() {
    const SECTIONS: [&str; 3] = ["## What happened", "## Why", "## How to fix"];
    let dir = repo().join("docs").join("diagnostics");
    let mut problems = Vec::new();
    for row in DIAGNOSTICS.iter().filter(|r| r.status.requires_emitter()) {
        let id = ident_of(row.class, row.number);
        let Ok(text) = std::fs::read_to_string(dir.join(format!("{id}.md"))) else {
            problems.push(format!("{id}: no docs/diagnostics/{id}.md"));
            continue;
        };
        if text.trim().is_empty() {
            problems.push(format!("{id}: the page is empty"));
            continue;
        }
        for section in SECTIONS {
            if !text.lines().any(|l| l.starts_with(section)) {
                problems.push(format!("{id}: no `{section}` section"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "these Live rows owe a complete page: {problems:?}. Write it from the emitter's own \
         message, not from the code's name -- three rows of this registry's first draft were \
         written the other way and disagreed with what the engine prints."
    );
}

/// **Check 3a** — every `Live` row is actually emitted.
///
/// The mirror of check 3b: 3b reds when a `Pending` row acquires an identifier use, this reds when
/// a `Live` row has none. Together they make the status field a claim about the tree rather than a
/// label, so a row cannot be flipped early and cannot be left behind.
#[test]
fn check_3a_every_live_row_has_at_least_one_emitter() {
    let c = gather();
    let orphans: Vec<String> = DIAGNOSTICS
        .iter()
        .filter(|r| r.status.requires_emitter())
        .map(|r| ident_of(r.class, r.number))
        .filter(|id| !has_token(&c.code, id))
        .collect();
    assert!(
        orphans.is_empty(),
        "these rows are Live but their identifiers appear nowhere in CODE: {orphans:?}. Either the \
         emitter was not written, or it names the code as a literal instead of the constant -- \
         which is the thing this registry exists to stop."
    );
}

/// **Check 4** — no undeclared literals.
///
/// Every `boyko-[BEW]dddd` written anywhere in source string literals or in the design corpus must
/// resolve to a registry row. This is the check that makes the registry authoritative rather than
/// advisory: a code invented at a call site fails the build.
#[test]
fn check_4_every_prefixed_literal_resolves_to_a_registry_row() {
    let c = gather();
    assert!(
        !c.literals.is_empty(),
        "no prefixed literals found at all; the walker is not reaching the sources"
    );

    let forward: BTreeSet<String> = FORWARD_DECLARED.iter().map(|(s, _)| (*s).to_string()).collect();

    let mut unresolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (class, number, where_) in &c.literals {
        if boyko_log::explain(*class, *number).is_none() {
            unresolved
                .entry(format!("{}{number:04}", *class as char))
                .or_default()
                .push(where_.clone());
        }
    }

    let surprises: Vec<&String> = unresolved.keys().filter(|k| !forward.contains(*k)).collect();
    assert!(
        surprises.is_empty(),
        "these codes are written as prefixed literals but carry no registry row and are not in \
         the forward-declared ledger: {surprises:?}. Either register the row, or add it to \
         FORWARD_DECLARED with the rung that lands it -- inventing a summary here is how three \
         rows of this registry came to disagree with the messages the engine prints."
    );

    // The ledger must SHRINK. A forward reference whose row has landed is stale, and a stale
    // ledger entry silently re-opens the hole it was carved to keep visible.
    let stale: Vec<&str> = FORWARD_DECLARED
        .iter()
        .filter(|(s, _)| {
            let class = s.as_bytes()[0];
            let n: u16 = s[1..].parse().expect("ledger entries are Cdddd");
            boyko_log::explain(class, n).is_some()
        })
        .map(|(s, _)| *s)
        .collect();
    assert!(
        stale.is_empty(),
        "these codes are now registered and must be REMOVED from FORWARD_DECLARED: {stale:?}"
    );
}

/// Codes named as prefixed literals in the design corpus whose registry rows land at a later rung.
///
/// **This is a ledger, not an allowlist**, and the distinction is the whole design. An allowlist
/// grows quietly and launders debt; this list is asserted in both directions — an unlisted
/// unresolved code reds, and a *listed* code that has since been registered also reds, so the list
/// can only shrink. Every entry names the rung that removes it.
///
/// They are not registered today because registering them means writing a summary, and a summary
/// invented from a code's name is exactly the defect this registry's first draft shipped in three
/// rows. Each lands with the rung that writes its emitter, where the message text exists to read.
const FORWARD_DECLARED: &[(&str, &str)] = &[
    ("W0102", "L3 — the aggregated per-drain drop report"),
    // ⚠️ `E0104` says L10 and L10-A did NOT land it. That is correct rather than overdue: L10's row
    // in the ladder scopes the rung to the DYNAMIC band, and `E0104` is the DOWNSTREAM band's boot
    // collision check — `define_target!`, which is L11a's. The ladder's "a later rung" and this
    // ledger's "L10" disagreed; the ladder is the one with the file list, so this row now names the
    // rung that actually builds the thing it reports on.
    // ⚠️ REPAIRED AT L10-A, and the repair is the point. This row read
    // *"L10 — dynamic target name arena exhausted"*, which is `E0106`'s condition wearing `E0105`'s
    // number: the corpus assigns `E0105` to `flush()`'s 2 s timeout in FOUR places
    // (`02-SINK-LIFECYCLE.md` ×4, `05-LADDER-GATES.md` §Integration 9) and never to the band.
    //
    // Left alone it would have been the L8c-C defect in the ledger next door: a row reserved for a
    // rung that has SHIPPED, describing work that rung never owed. And unlike a `Pending` registry
    // row, nothing here could have caught it — this ledger's shrink-only check fires when a listed
    // code becomes REGISTERED, and `E0105` is not going to be registered by the rung it named. It
    // would have sat here indefinitely, reading like debt.
    ("E0105", "L14 — flush() timed out waiting for the sink"),
    ("E0107", "L14 — sink open/close request refused"),
    // Likewise mis-summarised: the corpus's `E0108` is shutdown DETACHING after its bounded spin
    // (`02-SINK-LIFECYCLE.md`, `06-DISPOSITIONS.md` M16), not the control spec. The rung was right.
    ("E0108", "L14 — shutdown timed out and detached the sink thread"),
    ("E0109", "L15 — crash sink could not be opened"),
    ("W0110", "L14 — sink filter rejected every destination"),
    ("W0111", "L14 — census reports unsunk records"),
    ("W0112", "L13a — file rotation dropped records"),
    ("W0116", "L13b — binary sink site dictionary full"),
    ("W0117", "L16 — the ECS handoff ring overflowed"),
    ("E0118", "L15 — the panic hook could not complete a flush"),
];

// ────────────────────────────── checks 5 and 6, armed at L6 ───────────────────────────────

/// Everything that counts as **test code**, as `(path, text)`.
///
/// Two sources, and the second is an approximation stated rather than hidden:
///
/// 1. Files the walker already marks test-only — `tests/`, `benches/`, and the cross-file
///    `#[cfg(test)] mod` targets. `src/bin/` is excluded here although the walker marks it: a CLI
///    entry point is production code that happens to print, and letting it satisfy an
///    observed-by-a-test claim would be a hole.
/// 2. The tail of every other `.rs` file from its first `#[cfg(test)]` line. In this repository a
///    `#[cfg(test)] mod tests` is the last item in its file; where it is not, production text after
///    it joins the corpus. **The error direction is one-way**: a too-large corpus can only make
///    this check more permissive, never falsely red, so the approximation cannot manufacture a
///    failure — it can only fail to catch one, and it says so here.
///
/// Two files are excluded whatever they contain. `codes.rs` DEFINES every identifier and its own
/// test module asserts their classes, so including it would make every row observed by
/// construction. `code_registry.rs` — this file — names codes as **data**: check 0's sentinel is
/// `boyko-W1501`, which would silently satisfy the claim for that row. Both are the same exclusion
/// the CODE stream already makes, for the same reason.
///
/// # COMMENTS ARE STRIPPED, and that is a repair *(L8c)*
///
/// Until L8c this returned **raw text**, so a code named in a *comment* inside any test file
/// satisfied check 5. It was found the way these things are found: L8c wrote a comment in the
/// shared `tests/walker/` module explaining why `E3001` could not be excluded by a path rule, and
/// check 5 immediately reported that `E3001` *"is now named by a test and must be REMOVED from
/// untested_codes.txt"*. Nothing had tested it. A sentence about it existed.
///
/// The check's own failure text already concedes that **naming is a proxy for observing** — it
/// cannot tell an assertion from a mention. That is a stated weakness and an acceptable one. Being
/// satisfiable by *prose* is a different and worse thing: it means a gate about test coverage can
/// be discharged by documentation, in a file no test even runs.
///
/// So each file contributes `CODE ∪ LIT` rather than its raw bytes. LIT is kept deliberately: the
/// literal route (`#[should_panic(expected = "boyko-E…")]`, and a `watch(b'W', …)` beside a
/// message assertion) is a real observation, and dropping it would swap one false answer for
/// another.
fn test_corpus(root: &Path) -> Vec<(PathBuf, String)> {
    let files = rust_files(root);
    let marked = test_only_files(root, &files);
    let mut out = Vec::new();
    for f in &files {
        let s = f.to_string_lossy().replace('\\', "/");
        if s.contains("/src/bin/") || s.ends_with("/code_registry.rs") {
            continue;
        }
        if f.file_name().is_some_and(|n| n == "codes.rs") {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        // Strip comments before deciding what the file "names" -- see the doc above. The two
        // streams are concatenated because both routes to an observation are legitimate: the
        // identifier (`codes::E3001`) lives in CODE, the frozen literal (`"boyko-E3001"`) in LIT.
        //
        // NOTE the interaction with `split_streams`' own `#[cfg(test)]` stripping: for a file the
        // walker already MARKED test-only, that stripping would delete the very thing being looked
        // for, so the raw text is split but the region rule is irrelevant there -- a marked file is
        // test code in its entirety. For an unmarked file only the tail from its first
        // `#[cfg(test)]` is taken, and that tail is split the same way.
        // `split_streams`, NOT `production_code`: this corpus's SUBJECT is the test regions, and
        // the production rule would delete exactly what is being looked for. See
        // `walker::production_code` for the run where folding the two together reported 23 Live
        // codes as unobserved.
        let of = |text: &str| {
            let s = split_streams(text);
            let mut both = s.code;
            both.push('\n');
            both.push_str(&s.lit);
            both
        };
        if marked.contains(f) {
            out.push((f.clone(), of(&src)));
        } else if let Some(at) = src.find("#[cfg(test)]") {
            out.push((f.clone(), of(&src[at..])));
        }
    }
    out
}

/// Codes with no observing test, each naming why. A **data file**, so it is `.txt` and the `.rs`
/// scan cannot reach it — the self-referential trap v1's check 5 fell into.
const UNTESTED_LEDGER: &str = "tests/untested_codes.txt";

/// Parse the ledger into `(code, reason)`; `#` starts a comment.
fn untested_ledger() -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(UNTESTED_LEDGER);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is a required data file: {e}", path.display()));
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // An indented line CONTINUES the previous reason. Without this rule the parser would read
        // the second line of a wrapped reason as a code named "take" -- and the both-directions
        // assertions would then red on a ledger that is perfectly well formed.
        if line.starts_with(char::is_whitespace) {
            if let Some((_, reason)) = out.last_mut() {
                reason.push(' ');
                reason.push_str(t);
            }
            continue;
        }
        let (code, reason) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
        out.push((code.to_string(), reason.trim().to_string()));
    }
    out
}

/// **Check 5** — every `Live` `W`/`E` code is named by a test, or is in the ledger with a reason.
///
/// ARMED AT L6, which is where its header table said it would arm, and its first run found what a
/// first run is for: of twenty `Live` `W`/`E` rows, eight were named by no test at all. Five of
/// those were this rung's own and got observing tests; the rest are in the ledger.
///
/// **A code counts as named** if its identifier appears as a standalone token *or* its prefixed
/// literal (`boyko-W0103`) appears. The second form matters and is the stronger one: a test that
/// asserts the emitted TEXT is a better observation than one that names the constant, and
/// `file_sink_and_census.rs` is exactly that shape.
///
/// **What this check CANNOT claim, and it is in the failure text too**: naming is a proxy for
/// observing. It cannot tell a test that drives the condition and asserts the record from one that
/// mentions the code in a comment. It is the cheapest gate that makes an unobserved code
/// *countable*, and the ledger is what keeps the count honest — asserted in both directions, so it
/// can only shrink.
#[test]
fn check_5_every_live_warn_or_error_code_is_named_by_a_test() {
    let corpus = test_corpus(&repo());
    assert!(corpus.len() > 100, "only {} test files found; the corpus is not resolving", corpus.len());

    let named = |id: &str| {
        let literal = format!("boyko-{id}");
        corpus.iter().any(|(_, t)| has_token(t, id) || t.contains(&literal))
    };

    let live: Vec<String> = DIAGNOSTICS
        .iter()
        .filter(|r| r.status.requires_emitter() && matches!(r.class, b'W' | b'E'))
        .map(|r| ident_of(r.class, r.number))
        .collect();

    let ledger = untested_ledger();
    let listed: BTreeSet<&str> = ledger.iter().map(|(c, _)| c.as_str()).collect();

    let unlisted: Vec<&String> = live.iter().filter(|id| !named(id) && !listed.contains(id.as_str())).collect();
    assert!(
        unlisted.is_empty(),
        "these Live W/E codes are named by no test and are not in {UNTESTED_LEDGER}: {unlisted:?}. \
         Write a test that drives the condition and observes the record -- or, if the site cannot \
         be reached from a test, add a row naming exactly why. NAMING IS A PROXY FOR OBSERVING: \
         this check cannot tell a test that asserts the record from one that mentions the code."
    );

    // The ledger must SHRINK. A row whose code has since acquired a test is stale, and a stale row
    // silently re-opens the hole it was carved to keep visible -- the same both-directions rule
    // FORWARD_DECLARED carries, for the same reason.
    let stale: Vec<&String> = ledger
        .iter()
        .filter(|(c, _)| named(c))
        .map(|(c, _)| c)
        .collect();
    assert!(stale.is_empty(), "these codes are now named by a test and must be REMOVED from {UNTESTED_LEDGER}: {stale:?}");

    let unknown: Vec<&String> = ledger
        .iter()
        .filter(|(c, _)| !live.contains(c))
        .map(|(c, _)| c)
        .collect();
    assert!(unknown.is_empty(), "{UNTESTED_LEDGER} lists codes that are not Live W/E rows: {unknown:?}");

    for (code, reason) in &ledger {
        assert!(!reason.is_empty(), "{code} is in {UNTESTED_LEDGER} with no reason");
    }
}

/// **Check 6** — a `B` code is never an argument to an emission macro.
///
/// ARMED AT L6, and **re-specified against the tree at the same time.** The corpus says "panic-class
/// `B` codes appear only inside a `#[cold] fn … -> !` or a `panic!`". MEASURED: that is false of a
/// correct tree. `ScheduleBuildError` is deliberately dual-purpose — `ScheduleBuilder::build`
/// panics with `e.formatted()` while `try_build` returns the same error as an `Err` for a tool or
/// library caller — so `B9001`/`B9002`/`B9004`/`B9005` necessarily live in a `String`-returning
/// method that also feeds `Display`. Enforcing the literal rule would have required either deleting
/// the recoverable API or moving the codes back to string literals.
///
/// What the rule is *for* survives intact, and it is the corpus's own red state: **"emit a `B` code
/// from a `warn!`"**. A `B` code is a broken invariant; routing one through the emission macros
/// would make it a line in a log that the process then continued past.
///
/// **What this check CANNOT claim**: that every `B` code reaches a panic. It bounds the class of
/// site a `B` code may NOT appear in, which is the half that has a demonstrable failure.
#[test]
fn check_6_no_panic_code_is_an_argument_to_an_emission_macro() {
    const MACROS: [&str; 5] = ["warn!", "error!", "info!", "debug!", "trace!"];
    let c = gather();
    let mut offenders = Vec::new();
    for row in DIAGNOSTICS.iter().filter(|r| r.class == b'B') {
        let id = ident_of(row.class, row.number);
        let mut from = 0usize;
        while let Some(rel) = c.code[from..].find(&id) {
            let at = from + rel;
            from = at + 1;
            // Statement scope: from the last separator before the occurrence. A macro invocation
            // that has not been closed by then is the one this identifier sits inside.
            let start = c.code[..at].rfind([';', '{', '}']).map_or(0, |i| i + 1);
            let segment = &c.code[start..at];
            if MACROS.iter().any(|m| segment.contains(m)) {
                offenders.push(id.clone());
                break;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these B codes are passed to an emission macro: {offenders:?}. A B code is a broken \
         invariant -- routing one through warn!/error! makes it a line in a log that the process \
         then continues past. Its site is a panic."
    );
}

/// **Check 7** — every `LogTarget` impl resolves to the engine table.
///
/// The engine table is the only producer of `TargetId`s at this rung, so a hand-written
/// `impl LogTarget for …` outside `boyko_log`'s own `target.rs` is a target nobody registered —
/// and its control byte would be some other target's.
///
/// Test fixtures legitimately write such impls to drive the gates, which is exactly why CODE
/// excludes `tests/` and `benches/` wholesale rather than relying on an attribute those files do
/// not carry.
#[test]
fn check_7_every_log_target_impl_is_registered() {
    let c = gather();
    let occurrences = c.code.matches("impl LogTarget for").count()
        + c.code.matches("impl $crate :: target :: LogTarget for").count();
    assert_eq!(
        occurrences, 0,
        "found {occurrences} hand-written `impl LogTarget` in production code; engine targets \
         come from the `targets!` table, which is the only thing that mints an in-band TargetId"
    );
}

// ─────────────────────────────── the walker's own tests ───────────────────────────
//
// A walker is a program, and an untested one is a gate whose result nobody can trust. These
// exercise the stripping rules on inputs whose correct answer is written beside them.

#[test]
fn walker_strips_comments_and_literals_into_the_right_streams() {
    let src = r##"
        // a line comment with B1802
        /// a doc comment with B1802
        /* a block /* nested */ comment with B1802 */
        let s = "a literal with boyko-B1802";
        let r = r#"a raw literal with boyko-B9001"#;
        let c = 'x';
        let real = codes::B0002;
    "##;
    let s = split_streams(src);
    assert!(!has_token(&s.code, "B1802"), "comments and literals must not reach CODE");
    assert!(has_token(&s.code, "B0002"), "real identifiers must reach CODE");
    assert!(s.lit.contains("boyko-B1802"), "string contents must reach LIT");
    assert!(s.lit.contains("boyko-B9001"), "raw string contents must reach LIT");
}

#[test]
fn a_lifetime_is_not_mistaken_for_a_char_literal() {
    // `'a` opens no literal. Mistaking it for one swallows the rest of the file into LIT, which
    // would empty CODE and make every identifier check silently vacuous.
    let s = split_streams("fn f<'a>(x: &'a str) -> &'a str { let q = codes::W1501; x }");
    assert!(has_token(&s.code, "W1501"), "a lifetime must not swallow the rest of the file");
}

#[test]
fn token_matching_never_accepts_a_substring() {
    assert!(has_token("let a = B1802;", "B1802"));
    assert!(has_token("codes::B1802", "B1802"));
    assert!(!has_token("boyko-B1802", "B1802"), "the prefixed literal is not a token use");
    assert!(!has_token("XB1802", "B1802"));
    assert!(!has_token("B18020", "B1802"));
}

#[test]
fn literal_extraction_rejects_near_misses() {
    assert_eq!(prefixed_literals("boyko-B1802"), vec![(b'B', 1802)]);
    assert_eq!(prefixed_literals("boyko-W1501 and boyko-B0002"), vec![(b'W', 1501), (b'B', 2)]);
    assert!(prefixed_literals("boyko-X1802").is_empty(), "X is not a class");
    assert!(prefixed_literals("boyko-B180").is_empty(), "three digits is not a code");
    assert!(prefixed_literals("boyko-B18021").is_empty(), "five digits is not a code");
}

#[test]
fn the_cross_file_test_only_rule_marks_files_a_within_file_rule_misses() {
    // Measured in this tree: these two are reached by a `#[cfg(test)]`-gated `mod` in a DIFFERENT
    // file, so a within-file scan classifies them as production. Named explicitly, because the
    // rule's whole value is the 7 files it catches that the simpler rule does not.
    let root = repo();
    let files = rust_files(&root);
    let marked = test_only_files(&root, &files);
    for rel in [
        "crates/boyko_sdf_math/src/brick/tests.rs",
        "crates/boyko_physics/src/solver/colored_tests.rs",
    ] {
        let p = root.join(rel);
        if p.is_file() {
            assert!(marked.contains(&p), "{rel} must be marked test-only by the cross-file rule");
        }
    }
}
