//! The ONE scanner behind D3 C6's two halves (`docs/REFLECTION-PLAN-GATES.md`):
//! `tests/reflect_manifest_census.rs` (G1) asserts every enabling command line it finds
//! is a named row, and `tests/reflect_ci_coverage.rs` (G4) asserts the named rows equal
//! the found set. Two copies of this scan would let the halves drift apart — the same
//! defect class D12 refuses for the trybuild corpus — so both import THIS module via
//! `#[path]`. It is not a test target: cargo only turns top-level `tests/*.rs` files
//! into binaries, and this lives a directory down.
#![allow(dead_code)] // shared between two test binaries; each uses a subset.

use std::path::{Path, PathBuf};

/// The repository root — `CARGO_MANIFEST_DIR` IS the root for root-package tests.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Splits a feature entry / spec into its cross-package half, if it has one:
/// `"pkg/feat"` / `"pkg?/feat"` → `(pkg, weak, feat)`.
pub fn cross_entry(entry: &str) -> Option<(&str, bool, &str)> {
    let (pkg, feat) = entry.split_once('/')?;
    match pkg.strip_suffix('?') {
        Some(p) => Some((p, true, feat)),
        None => Some((pkg, false, feat)),
    }
}

/// A `--features` spec found by the scan, with where it was found.
pub struct Enabling {
    pub file: String,
    pub line: usize,
    pub spec: String,
}

/// Strips markdown/prose decoration from a token so `` `--features reflect`,`` reads as
/// its command-line self.
pub fn undecorate(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.' | ';' | '*' | '|' | '⇒' | '—' | '~')
    })
}

/// True when `spec` enables a reflect feature: `reflect` bare, or any `pkg/reflect`.
pub fn enables_reflect(spec: &str) -> bool {
    spec == "reflect" || cross_entry(spec).is_some_and(|(_, _, f)| f == "reflect")
}

/// Scans one file's text for `--features <spec>` occurrences whose spec enables a
/// `reflect` feature.
pub fn scan_text(file: &str, text: &str, found: &mut Vec<Enabling>) {
    for (idx, line) in text.lines().enumerate() {
        let mut tokens = line.split_whitespace().peekable();
        while let Some(raw) = tokens.next() {
            let tok = undecorate(raw);
            let spec_owned;
            let spec: &str = if tok == "--features" || tok == "-F" {
                match tokens.peek() {
                    Some(next) => {
                        spec_owned = undecorate(next).to_owned();
                        &spec_owned
                    }
                    None => continue,
                }
            } else if let Some(eq) = tok.strip_prefix("--features=") {
                eq
            } else {
                continue;
            };
            for part in spec.split(',') {
                if enables_reflect(part) {
                    found.push(Enabling {
                        file: file.to_owned(),
                        line: idx + 1,
                        spec: part.to_owned(),
                    });
                }
            }
        }
    }
}

/// Recursively scans a directory's files.
pub fn scan_dir(root: &Path, rel: &str, found: &mut Vec<Enabling>) {
    let dir = root.join(rel);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!(
            "C6 scan target `{rel}` does not exist or cannot be read -- a scan whose \
             subject silently vanished is a census that certifies nothing"
        );
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel_child = format!("{rel}/{}", entry.file_name().to_string_lossy());
        if path.is_dir() {
            scan_dir(root, &rel_child, found);
        } else if let Ok(bytes) = std::fs::read(&path) {
            scan_text(&rel_child, &String::from_utf8_lossy(&bytes), found);
        }
    }
}

/// The full C6 scan scope (GATES G1 C6): `.github/`, `scripts/`, and the gates plan
/// document itself.
pub fn scan_scope() -> Vec<Enabling> {
    let root = repo_root();
    let mut found = Vec::new();
    scan_dir(&root, ".github", &mut found);
    scan_dir(&root, "scripts", &mut found);
    let plan = root.join("docs").join("REFLECTION-PLAN-GATES.md");
    let text = std::fs::read_to_string(&plan)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", plan.display()));
    scan_text("docs/REFLECTION-PLAN-GATES.md", &text, &mut found);
    found
}

/// Every `--features` spec the scan scope may legitimately contain, parsed from
/// `tests/reflect_ci_coverage.rs`'s named list — the single source of truth (D3 C6).
pub fn named_specs() -> Vec<String> {
    let path = repo_root().join("tests").join("reflect_ci_coverage.rs");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "C6 has no named list: {} cannot be read ({e}). Absence is a RED, never a \
             SKIP -- without the list every enabling command line in the repo is \
             unaccounted for",
            path.display()
        )
    });
    let mut specs = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.contains("BEGIN REFLECT ENABLING SPECS") {
            inside = true;
            continue;
        }
        if line.contains("END REFLECT ENABLING SPECS") {
            inside = false;
            continue;
        }
        if inside && let Some(spec) = line.split('"').nth(1) {
            specs.push(spec.to_owned());
        }
    }
    assert!(
        !specs.is_empty(),
        "C6's named list in tests/reflect_ci_coverage.rs is empty or its BEGIN/END \
         delimiters are gone -- an empty reference list would let every enabling \
         invocation pass unnamed"
    );
    specs
}
