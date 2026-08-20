//! **`goldens/PINS.toml` is machine-read by one hand-rolled parser and by nothing else — so a
//! spelling that no conforming TOML parser accepts shipped, and every gate stayed green.**
//!
//! MEASURED 2026-08-20, on this file, at the particles-P1 rung: a pin appended with Windows paths
//! written `"D:\tmp\particle_sdf_collide.bmp"` — single backslashes. In a TOML *basic* string `\t`
//! is the TAB escape and `\p` is not an escape at all, so a conforming parser rejects **the whole
//! document**: all 31 pins that had nothing to do with the edit become unreadable in one stroke.
//! Nothing said so. `scripts/golden.ps1` reads this file with its own line scanner, which takes the
//! text between the quotes verbatim, so the bad spelling round-tripped through a full render, a
//! SHA-256 compare and a `PASS` — with the *decoded* path silently differing from the one every
//! sibling pin means.
//!
//! The failure class is the campaign's own: a datum with exactly one consumer, whose consumer is
//! lenient in precisely the direction the defect lies.
//!
//! # Why this is a hand check and not a real parser
//!
//! No workspace crate depends on a TOML parser. `toml v1.1.2` is in the lockfile ONLY as a
//! transitive dev-dependency of `trybuild` (`cargo tree -i toml`), and reaching it from here means
//! adding a direct third-party dependency to a workspace manifest — which
//! `docs/PARTICLES-PLAN.md`'s invariant 5 forbids outright ("No new third-party dependency"), and
//! which would be a large decision to take for a 60-line scanner.
//!
//! So this file checks the properties that have actually broken, and says plainly what it does not
//! do. **It is NOT a TOML parser and does not claim the file is valid TOML.** It claims four
//! things, each decidable by scanning:
//!
//! 1. every `\` inside a basic string begins a real TOML escape (the defect above);
//! 2. the file's UTF-8 BOM is exactly where it has always been — at byte 0, once (a conforming
//!    parser must be handed the text with it stripped, and a second one, or one landing mid-file
//!    through an append, is the same class of silent breakage);
//! 3. every line is a comment, a blank, a `[table]` header or a `key = value` (the mangled-append
//!    class — the P1 defect arrived through an append);
//! 4. a pin whose env names its own dump path names the SAME path the script deletes and hashes.
//!
//! The scanner's own assumptions are ASSERTED rather than assumed: the file uses no multi-line
//! (`"""`) strings and no literal (`'…'`) strings, both of which a line-oriented scanner would read
//! wrongly, and both of which this file checks for.
//!
//! # Non-vacuity
//!
//! [`escape_defect`] is unit-tested against synthetic good and bad inputs below, so the gate cannot
//! pass by never finding anything. That is the standing lesson of this campaign — a check whose
//! red is unreachable reports the same word as a check that works.
//!
//! # Why it lives in the root package
//!
//! `CARGO_MANIFEST_DIR` **is** the repository root here, so no `../..` walking can point the scan
//! at the wrong tree — `internal_docs_anchors.rs` / `engine_packages_census.rs`'s rationale,
//! verbatim. It is also a default member, so this runs in the plain `cargo test --workspace` battery
//! rather than behind a flag someone has to remember.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The UTF-8 byte-order mark this file has carried since it was created. Recorded, not removed:
/// `scripts/golden.ps1` (PowerShell) reads it happily, and stripping it is a change to an artifact
/// every pin's provenance runs through. A parser must be handed the text without it.
const UTF8_BOM: &str = "\u{feff}";

/// The escapes TOML 1.0 defines inside a *basic* (double-quoted) string, excluding the two
/// hex forms, which are handled by length.
const SIMPLE_ESCAPES: &[char] = &['b', 't', 'n', 'f', 'r', '"', '\\'];

/// Reads `goldens/PINS.toml` as text, WITH its BOM (the BOM is one of the things checked).
fn read_pins() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("goldens").join("PINS.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: {} must be readable: {e}", path.display()))
}

/// The first invalid escape in `line`, as `(column, description)`, or `None`.
///
/// A state machine over the line, not a regex: a `#` inside a basic string is not a comment, and a
/// `"` that follows a backslash does not close the string. Both distinctions are exactly the ones a
/// naive scan gets wrong on this file, whose values are Windows paths and whose comments are prose.
///
/// Columns are 1-based, to read like the parser error the defect would have produced.
fn escape_defect(line: &str) -> Option<(usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if !in_string {
            // Outside a string: `#` opens a comment that runs to end of line.
            if c == '#' {
                return None;
            }
            if c == '"' {
                in_string = true;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = false;
                i += 1;
            }
            '\\' => {
                let Some(&next) = chars.get(i + 1) else {
                    return Some((i + 1, "a backslash at end of line inside a string".into()));
                };
                if SIMPLE_ESCAPES.contains(&next) {
                    i += 2;
                } else if next == 'u' || next == 'U' {
                    let want = if next == 'u' { 4 } else { 8 };
                    let hex: String = chars[i + 2..].iter().take(want).collect();
                    if hex.chars().count() == want && hex.chars().all(|h| h.is_ascii_hexdigit()) {
                        i += 2 + want;
                    } else {
                        return Some((
                            i + 1,
                            format!("`\\{next}` must be followed by {want} hex digits, got `{hex}`"),
                        ));
                    }
                } else {
                    return Some((
                        i + 1,
                        format!(
                            "`\\{next}` is not a TOML escape. A Windows path in a basic string \
                             needs DOUBLE backslashes (`D:\\\\tmp\\\\x.bmp`), like every other pin \
                             in this file; `\\t` would silently decode to a TAB and `\\p` makes the \
                             WHOLE document unparseable"
                        ),
                    ));
                }
            }
            _ => i += 1,
        }
    }
    None
}

#[test]
fn every_basic_string_uses_a_valid_toml_escape() {
    // THE defect this file exists for. It is checked line by line so the failure names the line and
    // column a real parser would have named, rather than "the file is bad somewhere".
    let text = read_pins();
    let mut defects = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if let Some((col, why)) = escape_defect(line) {
            defects.push(format!("  line {} col {col}: {why}\n    {line}", n + 1));
        }
    }
    assert!(
        defects.is_empty(),
        "goldens/PINS.toml contains {} invalid string escape(s). A conforming TOML parser rejects \
         the ENTIRE document on the first one — every pin in the file, not just the edited one — \
         while `scripts/golden.ps1`'s hand-rolled reader takes the text verbatim and reports PASS:\n{}",
        defects.len(),
        defects.join("\n")
    );
}

#[test]
fn the_scanner_reads_this_file_s_shape_and_says_so() {
    // The scanner is LINE-oriented and treats every double quote as opening or closing a basic
    // string. Two TOML constructs would break that reading, and neither is used here — asserted,
    // so the assumption is checked rather than believed. If a future pin wants either, this test is
    // the one that must be answered first.
    let text = read_pins();
    assert!(
        !text.contains("\"\"\""),
        "goldens/PINS.toml has grown a multi-line basic string. The escape scanner in this file is \
         line-oriented and would read its interior as a sequence of one-line strings — teach it \
         multi-line strings before using one"
    );
    for (n, line) in text.lines().enumerate() {
        let code = line.split('#').next().unwrap_or("");
        assert!(
            !code.contains('\''),
            "goldens/PINS.toml line {} uses a literal (single-quoted) string outside a comment. \
             Literal strings have no escapes at all, so the scanner would mis-read a backslash \
             inside one:\n    {line}",
            n + 1
        );
    }
}

#[test]
fn the_bom_is_at_byte_zero_and_there_is_exactly_one() {
    // Recorded because it is load-bearing for whoever next tries a real parser: `tomllib` (and the
    // `toml` crate) reject this file AS-IS at line 1 column 1 purely because of the BOM, which has
    // been here since the file was created. A gate that failed to say so would look like the
    // escape defect and send the reader hunting in the wrong place.
    let text = read_pins();
    assert!(
        text.starts_with(UTF8_BOM),
        "goldens/PINS.toml lost its UTF-8 BOM. That is not itself an error, but it is a change to \
         an artifact every pin's provenance runs through, and `scripts/golden.ps1` reads this file \
         — make it deliberately or not at all"
    );
    assert_eq!(
        text.matches(UTF8_BOM).count(),
        1,
        "goldens/PINS.toml carries more than one BOM. A second one is what an append through a \
         BOM-writing tool leaves behind, and it is a parse error wherever it lands"
    );
}

#[test]
fn every_line_is_a_comment_a_table_header_or_a_key_value() {
    // The MANGLED-APPEND class: the P1 defect arrived by appending a block to this file, and an
    // append is also how a half-written table, a doubled header or a stray fragment would arrive.
    // A structural sweep catches those without pretending to be a parser.
    let text = read_pins();
    let mut bad = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_start_matches(UTF8_BOM).trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let ok = if line.starts_with('[') {
            line.ends_with(']') && line.len() > 2
        } else {
            // `key = value`, with the key non-empty and the value present.
            match line.split_once('=') {
                Some((k, v)) => !k.trim().is_empty() && !v.trim().is_empty(),
                None => false,
            }
        };
        if !ok {
            bad.push(format!("  line {}: {raw}", n + 1));
        }
    }
    assert!(
        bad.is_empty(),
        "goldens/PINS.toml has {} line(s) that are neither a comment, a table header nor a \
         `key = value`:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn a_pin_that_names_its_own_dump_path_names_the_one_it_hashes() {
    // `scripts/golden.ps1` DELETES the `bmp` path, runs the test with the `[pin.env]` table, then
    // asserts the file at `bmp` was freshly written and hashes it. If a pin's env names a DIFFERENT
    // `BOYKO_HOST_DUMP`, the render lands somewhere else and the script either aborts on a missing
    // file or — with a stale artifact from an earlier run present — hashes the wrong image.
    //
    // CONDITIONAL, and measured before it was written: 29 of the 32 pins name `BOYKO_HOST_DUMP` and
    // all 29 agree; three (`grand_showcase`, `deferred_sdf_only`, `deferred_mesh_only`) omit the key
    // and let their fixture's own default path stand. Omission is therefore legal and disagreement
    // is not.
    let text = read_pins();
    let mut bmp: BTreeMap<String, String> = BTreeMap::new();
    let mut dump: BTreeMap<String, String> = BTreeMap::new();
    let mut table = String::new();
    for raw in text.lines() {
        let line = raw.trim_start_matches(UTF8_BOM).trim();
        if line.starts_with('[') && line.ends_with(']') {
            table = line[1..line.len() - 1].to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        // The values here are basic strings whose only escape is `\\`; decoding that one is enough
        // to compare two paths, and `every_basic_string_uses_a_valid_toml_escape` above is what
        // guarantees no other escape is present to mis-decode.
        let value = v.trim_matches('"').replace("\\\\", "\\");
        if k == "bmp" {
            bmp.insert(table.clone(), value);
        } else if k == "BOYKO_HOST_DUMP" {
            dump.insert(table.trim_end_matches(".env").to_string(), value);
        }
    }
    assert!(!bmp.is_empty(), "no pin declared a `bmp` — the scan found nothing to check");
    for (pin, path) in &dump {
        let Some(declared) = bmp.get(pin) else {
            panic!("[{pin}.env] names BOYKO_HOST_DUMP but there is no [{pin}] with a `bmp`");
        };
        assert_eq!(
            declared, path,
            "pin `{pin}`: the script deletes and hashes `{declared}` but runs the test with \
             BOYKO_HOST_DUMP=`{path}`. The render lands at the second path and the gate reads the \
             first — which is either an abort or, with a stale file present, a PASS on an image \
             this run did not produce"
        );
    }
}

#[cfg(test)]
mod scanner_is_not_vacuous {
    use super::escape_defect;

    #[test]
    fn it_accepts_what_the_file_actually_contains() {
        assert_eq!(escape_defect(r#"bmp = "D:\\tmp\\x.bmp""#), None);
        assert_eq!(escape_defect("# a comment with a lone \\ and an apostrophe's quote"), None);
        assert_eq!(escape_defect(r#"key = "tab\there and a quote\" inside""#), None);
        assert_eq!(escape_defect(r#"key = "\u00e9 and \U0001F600""#), None);
        // A `#` INSIDE a string is not a comment: the scanner must keep checking past it.
        assert_eq!(escape_defect(r#"key = "a # not-a-comment \\ ok""#), None);
    }

    #[test]
    fn it_rejects_the_defect_that_shipped() {
        // The exact spelling that shipped at P1 and passed every gate.
        let d = escape_defect(r#"bmp = "D:\tmp\particle_sdf_collide.bmp""#);
        assert!(d.is_some(), "the P1 defect must be caught");
        // `\p` — not an escape at all, the half that makes the document unparseable.
        assert!(escape_defect(r#"bmp = "C:\path""#).is_some());
        // A truncated hex escape.
        assert!(escape_defect(r#"key = "\u12""#).is_some());
        // A trailing backslash inside an unterminated string.
        assert!(escape_defect(r#"key = "trailing \"#).is_some());
        // And a `#` inside a string must NOT hide a later defect from the scanner.
        assert!(escape_defect(r#"key = "a # then \q""#).is_some());
    }
}
