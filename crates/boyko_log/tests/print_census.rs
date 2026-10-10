//! **The gate the whole migration exists to make mechanical** *(L8c)*.
//!
//! L6, L7a, L7b, L8a and L8b each converted a batch of `println!`/`eprintln!` into log records and
//! each reported a count. Until this file, every one of those counts was a **claim**: the only
//! thing that had checked them was the person making them, and a site added tomorrow would land in
//! a workspace with nothing to notice. This walks `crates/*/src/**.rs` and fails on any print
//! outside `tests/print_allowlist.txt`.
//!
//! # The allowlist is a LEDGER and is checked in both directions
//!
//! An unlisted print reds — that is the obvious half. A **listed** file that no longer prints also
//! reds, which is the half that keeps the list honest: an allowlist that can only grow becomes a
//! place to put things, and within a few rungs it is the answer to "why is this print here?"
//! instead of the question. Every row names its file and its reason, and the list can only shrink
//! without a deliberate edit.
//!
//! # What this cannot see, stated because a reader will assume otherwise
//!
//! It matches the four macros by name. It does **not** see `stdout().write_all`, `io::Write` on a
//! raw handle, or `libc::write` — the same three the corpus already records as beyond the reach of
//! clippy's `disallowed-macros`. A crate that wanted to print past this gate could; the gate is
//! against drift, not against an adversary.
//!
//! # The walker is shared, not copied
//!
//! `mod walker` is the same module `code_registry.rs` uses, so the cross-file `#[cfg(test)] mod`
//! rule and the `src/bin/` exclusion are written once. Two copies would be two answers to "is this
//! file production code?", and the day they disagreed neither failure message would say which was
//! right.

mod walker;

use std::collections::BTreeSet;
use std::path::PathBuf;

use walker::{production_code, repo, rust_files, test_only_files};

/// The allowlist, relative to this crate's manifest directory.
const ALLOWLIST: &str = "tests/print_allowlist.txt";

/// The four macros. Split so this file does not match itself — its own prose names all four, and a
/// census that reddened on its own documentation would be the first thing anyone deleted.
const PRINT_MACROS: &[&str] = &[
    concat!("print", "ln!"),
    concat!("eprint", "ln!"),
    concat!("print", "!"),
    concat!("eprint", "!"),
];

/// One allowlisted file and the reason it is there.
struct AllowRow {
    /// Repo-relative, forward-slashed.
    path: String,
    reason: String,
}

/// Parse `print_allowlist.txt`: `path  reason`, one per line, `#` comments, continuations indented.
fn allowlist() -> Vec<AllowRow> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ALLOWLIST);
    let text = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", p.display()));
    let mut rows: Vec<AllowRow> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        // A continuation line is indented; it extends the previous row's reason.
        if line.starts_with(char::is_whitespace) {
            if let Some(last) = rows.last_mut() {
                last.reason.push(' ');
                last.reason.push_str(line.trim());
            }
            continue;
        }
        let mut it = line.splitn(2, char::is_whitespace);
        let path = it.next().unwrap_or("").trim().to_string();
        let reason = it.next().unwrap_or("").trim().to_string();
        rows.push(AllowRow { path, reason });
    }
    rows
}

/// Every production `.rs` file that contains at least one print macro in its CODE stream.
///
/// CODE, not raw text — a print named in a comment is a record of one that was removed, and this
/// campaign has already had one gate red on its own history.
fn printing_files() -> (BTreeSet<String>, usize) {
    let root = repo();
    let files = rust_files(&root);
    let excluded = test_only_files(&root, &files);

    let mut hits = BTreeSet::new();
    let mut scanned = 0usize;
    for f in &files {
        if excluded.contains(f) {
            continue;
        }
        let rel = f
            .strip_prefix(&root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        // `src/` only. `tests/` and `benches/` are already excluded by the walker; this keeps a
        // stray top-level file from counting as a crate source.
        if !rel.contains("/src/") && !rel.starts_with("src/") {
            continue;
        }
        scanned += 1;
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let code = production_code(&src);
        if PRINT_MACROS.iter().any(|m| code.contains(m)) {
            hits.insert(rel);
        }
    }
    (hits, scanned)
}

#[test]
fn no_production_source_prints_outside_the_allowlist() {
    let (printing, scanned) = printing_files();

    // VACUITY GUARD. A walk that resolved the wrong root, or an exclusion rule that swallowed the
    // workspace, would produce an empty `printing` set and a green gate that had checked nothing.
    // The engine has hundreds of production sources; a scan that finds a handful is broken.
    assert!(
        scanned > 200,
        "vacuity guard: only {scanned} production sources were scanned, so a green here would \
         mean the walk resolved the wrong root rather than that the workspace is clean"
    );

    let allowed: BTreeSet<String> = allowlist().into_iter().map(|r| r.path).collect();
    let offenders: Vec<&String> = printing.iter().filter(|p| !allowed.contains(*p)).collect();

    assert!(
        offenders.is_empty(),
        "these production sources print outside `{ALLOWLIST}`: {offenders:#?}\n\
         \n\
         The engine logs through `boyko_log`. A print is not a diagnostic: it has no level, no \
         target, no code, no rate policy and no sink — so it cannot be turned off, cannot be \
         routed, and cannot be found by anyone who does not already know it exists. Migrate the \
         site (`warn!`/`error!` with a registry code, or `info!` with none), or add a row to the \
         allowlist NAMING THE REASON. {scanned} production sources scanned."
    );
}

#[test]
fn every_allowlist_row_still_has_a_print() {
    // The direction that keeps the ledger from becoming a dumping ground. A row whose file stopped
    // printing is a row nobody will reread, and the next person adds theirs beside it.
    let (printing, _) = printing_files();
    let rows = allowlist();
    assert!(
        !rows.is_empty(),
        "vacuity guard: the allowlist parsed to zero rows, so both directions of this check are \
         trivially satisfied"
    );

    let stale: Vec<&AllowRow> = rows.iter().filter(|r| !printing.contains(&r.path)).collect();
    let names: Vec<&String> = stale.iter().map(|r| &r.path).collect();
    assert!(
        stale.is_empty(),
        "these allowlist rows name files that no longer print: {names:#?}. Delete the row. An \
         allowlist that can only grow stops being a ledger and becomes a place to put things."
    );
}

#[test]
fn every_allowlist_row_carries_a_reason() {
    // A path with no reason is the allowlist-laundering this design says it prevents: it records
    // WHAT is excused and not WHY, and the next reader cannot tell a decision from an oversight.
    let rows = allowlist();
    let bare: Vec<&String> = rows.iter().filter(|r| r.reason.len() < 20).map(|r| &r.path).collect();
    assert!(
        bare.is_empty(),
        "these allowlist rows carry no reason (or too short a one): {bare:#?}. The reason is the \
         row's whole content — the path is just where to find it."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parser_reads_a_row_its_continuation_and_ignores_comments() {
        // The RED for the parser itself, run as a unit. Without it the two directions above could
        // both pass because the allowlist parsed to nothing useful — which the vacuity guards
        // catch for emptiness but not for a mis-split path/reason.
        let rows = allowlist();
        assert!(rows.len() >= 3, "the L6 and L8b fallbacks alone are four rows: {}", rows.len());
        for r in &rows {
            assert!(r.path.ends_with(".rs"), "a row's first token is a path: {:?}", r.path);
            assert!(!r.path.contains(' '), "a path is one token: {:?}", r.path);
            assert!(r.reason.len() >= 20, "row {:?} has no reason", r.path);
        }
    }
}
