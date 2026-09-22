//! CORE C2 gate 3 — `MAX_COMPONENTS` is IMPORTED, never redeclared (CORE D5).
//!
//! A source census over this crate's `src/**.rs`: a local `const MAX_COMPONENTS` or a
//! bare `512` literal in an array-length position is the drift CORE C2's first RED
//! mutation demonstrates — a 512-slot table over a smaller kernel id space, with
//! **nothing left to red** when the kernel bound moves. The census is what makes the
//! rule mechanical rather than reviewed.

// Miri refuses host file I/O under isolation (measured at GATES G4's fifth RED:
// `CreateFileW not available when isolation is enabled`) — and this census is a
// source-text scan, which Miri has no business re-running anyway.
#![cfg(not(miri))]

use std::path::PathBuf;

/// Collects every `.rs` under this crate's `src/`, recursively.
fn source_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("crate src/ is readable") {
        let path = entry.expect("readable dir entry").path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// True when `line` uses `512` as an array LENGTH — the `[T; 512]` form: the previous
/// non-whitespace character before the literal is `;`.
fn is_array_len_512(line: &str) -> bool {
    let mut rest = line;
    while let Some(pos) = rest.find("512") {
        let before = &rest[..pos];
        if before.trim_end().ends_with(';') {
            return true;
        }
        rest = &rest[pos + 3..];
    }
    false
}

/// The census: no redeclaration, no bare bound — and non-vacuously, the import IS
/// there (a scan whose subject can vanish while the scan stays green is not a check).
#[test]
fn max_components_is_imported_never_redeclared() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&src, &mut files);
    assert!(!files.is_empty(), "the census found no source files -- wrong directory?");

    let mut offences = Vec::new();
    let mut import_seen = false;
    for path in &files {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        for (idx, line) in text.lines().enumerate() {
            if line.contains("const MAX_COMPONENTS") {
                offences.push(format!(
                    "{}:{}: local `const MAX_COMPONENTS` -- CORE D5 says IMPORTED, \
                     never redeclared; a local copy sizes the table against a bound \
                     the kernel no longer has, and nothing reds when they drift",
                    path.display(),
                    idx + 1
                ));
            }
            if is_array_len_512(line) {
                offences.push(format!(
                    "{}:{}: bare `512` in an array-length position -- the bound's \
                     one carrier is the `MAX_COMPONENTS` import (CORE D5)",
                    path.display(),
                    idx + 1
                ));
            }
            if line.contains("use boyko_ecs::") && line.contains("MAX_COMPONENTS") {
                import_seen = true;
            }
        }
    }
    assert!(offences.is_empty(), "MAX_COMPONENTS drift hazards:\n{}", offences.join("\n"));
    assert!(
        import_seen,
        "NON-VACUITY: no `use boyko_ecs::…MAX_COMPONENTS` import found in src/ -- the \
         census's subject has vanished, so its green certifies nothing (did the registry \
         move, or did the import get replaced by something this scan cannot see?)"
    );
}
