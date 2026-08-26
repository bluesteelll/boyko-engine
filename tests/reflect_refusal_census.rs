//! **CORE C9 gate 2 (D36) — the anti-rot census over the derive's refusals, and it is
//! BIDIRECTIONAL or it is nothing.**
//!
//! `#[component(reflect)]`'s refusals are only as honest as the list that enumerates them:
//! *a refusal outside `REFUSALS` is structurally invisible to the census that keeps
//! refusals honest* (D20 item 2). This file is that census, and it closes both directions.
//!
//! # The instrument, and why it is a source-text scan
//!
//! `boyko_macros` is `[lib] proc-macro = true`, so **no test can import anything from it**
//! — D31 measured that same obstacle about that same crate. A source-text scan is the only
//! available instrument across a proc-macro boundary, and it is the shape
//! `tests/internal_docs_anchors.rs` already uses in this repository.
//!
//! # What C9's gate 2 asked for first, and why neither half was constructible
//!
//! It asked for *"a `const REFUSALS: &[&str]` the derive itself iterates"* and a test
//! comparing its length to a fixture count.
//!
//! * No test can read the const (above).
//! * Nothing *iterates* a list of rule names to decide anything: each refusal is a distinct
//!   syntactic condition evaluated at its own site. A `const` the derive merely
//!   **declares** is computed and never read — the dead-datum class this campaign has now
//!   found five times.
//! * And an equality of counts is **one-directional**. It sees a rule added without a
//!   fixture. It cannot see the failure D20 item 2 exists to name — a refusal added to the
//!   derive and *not* to `REFUSALS` — because a `&[&str]` carries neither span nor message,
//!   so each refusal's text lives at its own site and a new site can simply not appear.
//!   That is C8 gate 5's shape: a drift test that cannot detect drift by construction.
//!
//! # The shape that makes the datum live
//!
//! `const REFUSALS: &[(&str, &str)]` — rule name, message — with one `IDX_*` per rule, and
//! every refusal site emitting `REFUSALS[IDX_X].1` inside its `quote_spanned!`. Then:
//!
//! * a rule added here **without its fixture** reds [`every_refusal_row_has_its_fixture`];
//! * a refusal site added to the derive **without a row** does not *compile*, because there
//!   is no `IDX_` to name and no message to emit — the direction the struck form could not
//!   see at all;
//! * a row whose message drifts from what the compiler actually prints reds
//!   [`every_blessed_stderr_carries_its_own_rows_message`], which is the clause that ties
//!   row, fixture and real compiler output into one chain.
//!
//! # Scope: DIAGNOSTIC quality, not termination (D21, restated)
//!
//! This census is **not** the acyclicity proof and never was. That is `validate`'s
//! `NestedCycle` arm, with `NestedNotInline` beside it for addressing-validity, both landed
//! at C6. The consequence of a missing refusal here is a worse diagnostic, not an unsound
//! descend.
//!
//! It also deliberately does **not** assert that the derive names the standard indirection
//! kinds. That list was struck at D34: it is a dead datum (the `Opaque`-field row already
//! reaches the same verdict at the same span — measured) and it is not a sound detector
//! (`is_nested_path` decides on *"has generic arguments anywhere"*, so a user's `MyArena<T>`
//! is syntactically identical to `Vec<T>`). Adding it back would re-create the drift surface
//! D21 removed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The derive that owns every `compile_error!` this census counts.
const DERIVE_SRC: &str = "crates/boyko_macros/src/reflect.rs";

/// The crate that owns the ONE **message-only** row — a trait bound, not a `compile_error!`.
const REFLECT_TRAIT_SRC: &str = "crates/boyko_reflect/src/reflect.rs";

/// The census directory: one `.rs` fixture per `REFUSALS` row, and nothing else.
const CORPUS_DIR: &str = "crates/reflect_fixture/tests/reflect_compile_fail";

/// The upstream pins — fixtures for refusals C9 does NOT author (D34). Deliberately
/// outside the census, and this file asserts they stay outside.
const UPSTREAM_DIR: &str = "crates/reflect_fixture/tests/reflect_compile_fail_upstream";

/// The rule name of the one row whose refusal is a trait bound rather than an emission.
const MESSAGE_ONLY_ROW: &str = "missing_default_rejected";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One scanned source's **code**, with every Rust comment removed.
///
/// ⚠️ **Every clause below is a text scan, and a text scan that reads comments is
/// satisfiable by a comment.** MEASURED after C9 landed: delete the union refusal site
/// outright and this census reds correctly, but delete it and leave
/// `// TODO: emit REFUSALS[IDX_UNION].1 here once C10 lands.` in its place and the census
/// reports **7 passed, exit 0** with the derive no longer refusing a union. That is not a
/// weaker version of the defect [`every_index_constant_exists_and_is_read`]'s first form
/// had — it is the same defect wearing the repair, because keying on `.1` only changed
/// which spelling a comment has to use. The corpus's feature-ON leg still catches it; this
/// census is the half that runs in the default feature-OFF sweep, and there it was green.
///
/// So the census reads [`code_only`]'s output rather than the file's bytes, for **all
/// three** scanned things — the `REFUSALS` table, the `IDX_` reads, and the
/// `on_unimplemented` message — because the class is the instrument's, not one clause's.
fn read_code(rel: &str) -> String {
    let p = repo_root().join(rel);
    let raw =
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
    code_only(&raw, rel)
}

/// Returns `source` with every Rust comment removed, so a clause reads **code** and never
/// **prose**. `whence` names the file, for the refusal messages.
///
/// # This is a SECOND COPY of `reflect_absence_census.rs`'s `code_only`, deliberately
///
/// That file is where this scanner was first written (C8 follow-up), and it is not shared
/// for two reasons, one mechanical and one about what sharing buys.
///
/// *Mechanical:* the function is a census-checked citation target —
/// `docs/REFLECTION-PLAN-CORE.md` cites it by `file:line`, and the same block records the
/// SHA-256 the file was restored to after that rung's RED. Moving it out would invalidate
/// the citation and make the recorded hash read as stale, and would additionally couple two
/// packages' test trees through a `../../../` `#[path]`. That is real blast radius for a
/// leaf primitive.
///
/// *What sharing buys, and why it is not needed here:* `tests/reflect_scan_support/mod.rs`
/// is shared because its two consumers are **two halves of one gate** — if they scanned
/// differently, one half would stop seeing what the other reports, and the hole would be a
/// GREEN. These two censuses are independent gates that happen to need the same primitive.
/// A divergence between the copies cannot open a green hole in either, because this
/// scanner **refuses rather than mis-reads**: the three constructs it does not model panic
/// with instructions, so a copy that lags behind the other fails loudly at the file that
/// grew the construct. And each copy carries its own pin test
/// ([`the_scanner_reads_code_and_not_prose`] here), so neither is an unpinned datum.
///
/// # What it models, and what it REFUSES rather than mis-reads
///
/// Line comments (`//`, `///`, `//!`) and plain string literals with backslash escapes —
/// which is every construct the two scanned sources contain today, re-checked on every run
/// rather than assumed. It deliberately does not model block comments, raw strings, or a
/// `'"'` character literal; each is detected *in the normal state*, so a mention inside a
/// comment cannot trip a guard, and each panics saying what to do. A silent mis-scan of a
/// construct the scanner does not understand is the failure this refusal exists to prevent:
/// it would drop code text, and dropped code text is a GREEN on every clause below.
fn code_only(source: &str, whence: &str) -> String {
    let src: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut in_string = false;
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(&next) = src.get(i + 1) {
                    out.push(next);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        // ── normal state ─────────────────────────────────────────────────────────────
        if c == '/' && src.get(i + 1) == Some(&'/') {
            // Drop to end of line; the newline itself is pushed by the next iteration, so
            // line structure survives and a stripped line cannot join its neighbour.
            while i < src.len() && src[i] != '\n' {
                i += 1;
            }
            continue;
        }
        assert!(
            !(c == '/' && src.get(i + 1) == Some(&'*')),
            "{whence} opens a BLOCK comment, which `code_only` does not model -- it would be \
             copied into the scanned text and could satisfy a clause below exactly like the \
             prose this function exists to exclude. Use `//` comments in that file, or teach \
             this scanner block comments in the same change."
        );
        if c == '"' {
            let quoted_char = i > 0 && src[i - 1] == '\'' && src.get(i + 1) == Some(&'\'');
            assert!(
                !quoted_char,
                "{whence} contains a `'\\\"'` character literal, which `code_only` would \
                 mistake for the start of a string and then scan the rest of the file in \
                 string state -- every comment after it would be copied into the scanned \
                 text. Rewrite it, or teach this scanner character literals."
            );
            let mut start = i;
            while start > 0 && src[start - 1] == '#' {
                start -= 1;
            }
            assert!(
                !(start > 0 && src[start - 1] == 'r'),
                "{whence} contains a RAW string literal, which `code_only` does not model -- \
                 its `\\` is not an escape, so the scanner would leave string state at the \
                 wrong quote and mis-classify everything after it. Rewrite it, or teach this \
                 scanner raw strings."
            );
            in_string = true;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Decodes a Rust string literal's body — the escapes this table actually uses.
///
/// The table's messages are written with `\`-newline continuations so the source stays
/// inside the line budget, and Rust strips the newline plus the following indentation. A
/// scan that compared raw source bytes would therefore be comparing something no compiler
/// ever prints, which is the failure mode this helper exists to avoid: the census must read
/// the string the way rustc reads it, or the `.stderr` clause below is comparing two
/// different things and passing.
fn unescape(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut it = body.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            // `\` + newline: the newline AND the following leading whitespace vanish.
            Some('\n') => {
                while it.peek().is_some_and(|c| c.is_whitespace()) {
                    it.next();
                }
            }
            Some('\r') => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                while it.peek().is_some_and(|c| c.is_whitespace()) {
                    it.next();
                }
            }
            other => panic!("unsupported escape `\\{other:?}` in a REFUSALS message"),
        }
    }
    out
}

/// Reads one `"..."` literal starting at `bytes[i] == '"'`, returning `(decoded, next_i)`.
fn take_literal(s: &str, quote: usize) -> (String, usize) {
    let b = s.as_bytes();
    assert_eq!(b[quote], b'"');
    let mut i = quote + 1;
    let start = i;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => break,
            _ => i += 1,
        }
    }
    assert!(i < b.len(), "unterminated string literal in {DERIVE_SRC}");
    (unescape(&s[start..i]), i + 1)
}

/// One parsed `REFUSALS` row.
#[derive(Debug)]
struct Row {
    rule: String,
    message: String,
}

/// Parses the `REFUSALS` table out of the derive's source.
///
/// A parser rather than a `contains` scan, because the clauses below need the row's
/// **bytes** — the message is what the compiler prints, and comparing anything else would
/// make this census green against a table nobody reads.
fn refusals() -> Vec<Row> {
    let src = read_code(DERIVE_SRC);
    let decl = "pub(crate) const REFUSALS: &[(&str, &str)] = &[";
    let start = src
        .find(decl)
        .unwrap_or_else(|| panic!("{DERIVE_SRC} no longer declares `REFUSALS` -- the census has no subject"))
        + decl.len();
    let end = src[start..]
        .find("\n];")
        .unwrap_or_else(|| panic!("`REFUSALS` in {DERIVE_SRC} is not terminated by `\\n];`"))
        + start;
    let table = &src[start..end];

    let mut rows = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = table[i..].find('"') {
        let q = i + rel;
        let (rule, after) = take_literal(table, q);
        let rel2 = table[after..]
            .find('"')
            .unwrap_or_else(|| panic!("row `{rule}` has a name but no message literal"));
        let (message, after2) = take_literal(table, after + rel2);
        rows.push(Row { rule, message });
        i = after2;
    }
    rows
}

/// The `.rs` fixture stems in `dir`, sorted.
fn fixture_stems(dir: &str) -> BTreeSet<String> {
    let p = repo_root().join(dir);
    std::fs::read_dir(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

// ─────────────────────────────── the clauses ────────────────────────────────

/// **[`code_only`] is this census's instrument, so it is pinned rather than trusted.**
///
/// An instrument only the gate exercises is checked exactly as far as today's sources
/// happen to reach, and *a datum computed and asserted nowhere* is this campaign's
/// most-repeated defect. The five shapes below are the ones the clauses turn on: a
/// whole-line comment (the measured mutation's shape), a **trailing** comment (what a
/// line-oriented stripper would miss), real code, a `//` inside a string literal (which
/// must NOT be taken for a comment, or the rest of a code line vanishes), and an escaped
/// quote inside a string (which must NOT close it, or every comment after it survives).
///
/// The last two are the directions that fail *green*: both drop or admit text silently,
/// and a clause reading the wrong text passes.
#[test]
fn the_scanner_reads_code_and_not_prose() {
    const TOKEN: &str = "REFUSALS[IDX_UNION].1";
    const WHENCE: &str = "<pin test>";

    let whole_line = format!("// TODO: emit {TOKEN} here once C10 lands.\nlet x = 1;\n");
    assert!(!code_only(&whole_line, WHENCE).contains(TOKEN), "a whole-line comment survived");
    assert!(code_only(&whole_line, WHENCE).contains("let x = 1;"), "the code line did not survive");

    let trailing = format!("let x = 1; // beside {TOKEN}\n");
    assert!(!code_only(&trailing, WHENCE).contains(TOKEN), "a TRAILING comment survived");

    let code = format!("let msg = spanned_message({TOKEN}, span);\n");
    assert!(code_only(&code, WHENCE).contains(TOKEN), "a real refusal site was stripped");

    let in_string = "let s = \"a // b\"; let t = 1;\n";
    assert!(
        code_only(in_string, WHENCE).contains("let t = 1;"),
        "a `//` inside a string literal was taken for a comment, so the rest of the line was \
         dropped -- that direction is a silent GREEN"
    );

    // `REFUSALS`'s own messages carry `\"bitset\"`, so this is not a hypothetical shape.
    let escaped = "let m = \"say \\\"hi\\\" now\"; // TODO: REFUSALS[IDX_UNION].1\n";
    let scanned = code_only(escaped, WHENCE);
    assert!(
        scanned.contains("say \\\"hi\\\" now") && !scanned.contains(TOKEN),
        "an escaped quote closed the string early, so the trailing comment was scanned as \
         code -- the exact direction this scanner exists to close"
    );
}

/// **Non-vacuity, first.** Every clause below is a comparison, and a comparison of two
/// empty sets is green.
#[test]
fn the_census_has_a_subject() {
    let rows = refusals();
    assert!(!rows.is_empty(), "`REFUSALS` parsed as EMPTY -- every clause below is vacuous");
    let stems = fixture_stems(CORPUS_DIR);
    assert!(!stems.is_empty(), "the corpus directory holds no `.rs` fixture");
    assert!(
        !fixture_stems(UPSTREAM_DIR).is_empty(),
        "the upstream-pin directory is empty, so the clause asserting those pins stay OUT \
         of the census has nothing to exclude"
    );
}

/// **The bijection, both directions in one assertion.**
///
/// A row without its fixture is a refusal nothing pins; a fixture without its row is a
/// refusal the census cannot see, which is the exact defect D20 item 2 names. Keyed by
/// NAME rather than by count: an equality of counts is satisfied by a rename, and a rename
/// is how a fixture quietly stops testing the rule it is filed under.
#[test]
fn every_refusal_row_has_its_fixture() {
    let rows = refusals();
    let named: BTreeSet<String> = rows.iter().map(|r| r.rule.clone()).collect();
    assert_eq!(
        named.len(),
        rows.len(),
        "`REFUSALS` has two rows with the same rule name; one of them can never have its \
         own fixture"
    );
    let stems = fixture_stems(CORPUS_DIR);

    let rows_without_fixture: Vec<&String> = named.difference(&stems).collect();
    let fixtures_without_row: Vec<&String> = stems.difference(&named).collect();

    assert!(
        rows_without_fixture.is_empty() && fixtures_without_row.is_empty(),
        "the refusal corpus and `REFUSALS` have drifted.\n  \
         rows with no fixture in {CORPUS_DIR}: {rows_without_fixture:?}\n  \
         fixtures with no row in {DERIVE_SRC}: {fixtures_without_row:?}\n\n\
         A row with no fixture is a refusal nothing pins. A fixture with no row is a \
         refusal this census is blind to -- the defect D20 item 2 exists to close."
    );
}

/// Every census fixture carries a blessed `.stderr`.
///
/// trybuild would red on a missing one, but only in the leg that runs it; this says so
/// from outside the feature, in the default `cargo test -p boyko-engine` sweep.
#[test]
fn every_census_fixture_is_blessed() {
    for stem in fixture_stems(CORPUS_DIR) {
        let p = repo_root().join(CORPUS_DIR).join(format!("{stem}.stderr"));
        assert!(Path::new(&p).is_file(), "fixture `{stem}` has no blessed `.stderr`");
    }
}

/// **The clause that ties the row to what the compiler actually prints.**
///
/// The row's message is not a label for the rule; it **is** the diagnostic's bytes, emitted
/// at the refusal site as `REFUSALS[IDX_X].1`. So the blessed output has to contain it, and
/// a row edited without a re-bless — or a site that stopped reading the table and inlined
/// its own text — reds here.
///
/// The `message-only` row is included, and it is the reason this clause matches SEGMENTS
/// rather than the whole string. `on_unimplemented` messages are **templates**: rustc
/// substitutes `{Self}` with the offending type's name, so the row's bytes and the printed
/// bytes differ by exactly the placeholders. MEASURED while writing this clause — the first
/// form was a plain `contains` and it reddened on that row, correctly. Every literal
/// segment between placeholders must appear, in order, which keeps the drift the clause
/// exists to catch (a row edited without a re-bless) fully visible while allowing the one
/// substitution rustc is entitled to make.
#[test]
fn every_blessed_stderr_carries_its_own_rows_message() {
    for row in refusals() {
        let p = repo_root().join(CORPUS_DIR).join(format!("{}.stderr", row.rule));
        let blessed = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));

        // Split the row's message on `{…}` placeholders; a message with none is one
        // segment and the check degenerates to the plain `contains` it should be.
        let mut segments: Vec<&str> = Vec::new();
        let mut rest = row.message.as_str();
        while let Some(open) = rest.find('{') {
            let close = rest[open..]
                .find('}')
                .unwrap_or_else(|| panic!("row `{}` has an unterminated `{{`", row.rule))
                + open;
            segments.push(&rest[..open]);
            rest = &rest[close + 1..];
        }
        segments.push(rest);

        let mut cursor = 0usize;
        for seg in segments.iter().filter(|s| !s.is_empty()) {
            let found = blessed[cursor..].find(seg).unwrap_or_else(|| {
                panic!(
                    "`{}`'s blessed `.stderr` does not carry its own `REFUSALS` message.\n\n  \
                     row says: {}\n  missing segment: {seg}\n\n\
                     Either the row was edited without re-blessing, or the refusal site \
                     stopped emitting `REFUSALS[IDX_...].1` and inlined its own text -- \
                     which is the table becoming a dead datum again.",
                    row.rule, row.message
                )
            });
            cursor += found + seg.len();
        }
    }
}

/// **Every row has an `IDX_` constant, and every EMITTED row is read AT A SITE.**
///
/// The index constants and the table are two lists ordered by convention; the derive pins
/// the pairing at const-eval with `same_str(REFUSALS[IDX_…].0, "…")`. This clause covers
/// what that pin cannot see: a row whose refusal is never emitted.
///
/// ⚠️ **The first form of this clause counted mentions, and RED 5 showed it could not
/// fire.** *"At least two occurrences — one declaration, one use"* is satisfied by the
/// declaration plus the const-eval pin, so deleting the whole `quote_spanned!` site left it
/// green (MEASURED: the census reported 7 passed while `bitset_storage_rejected` compiled).
/// The distinguishing byte is the tuple element: the name pin reads `.0`, and only a
/// refusal SITE reads `.1`, which is D36's *"every refusal site emitting
/// `REFUSALS[IDX_X].1` inside its `quote_spanned!`"* said in a form a scan can check.
///
/// The one **message-only** row is exempt from the site half and from nothing else: its
/// refusal is a trait bound in another crate, so there is no site to find — which is why
/// [`the_message_only_row_matches_the_on_unimplemented_attribute`] exists and why the
/// exemption is spelled here rather than inferred from a missing match.
#[test]
fn every_index_constant_exists_and_is_read() {
    // A rule's fixture name and its index constant are two vocabularies, and one is not
    // derivable from the other: `vec_field_rejected`'s rule is the `Opaque`-FIELD rule
    // (D34 merged the `Vec`/`Box`/`Option`/`PhantomData`/`&T`/raw-pointer row into it), so
    // a mechanical uppercase would name a constant the derive deliberately does not have.
    // The pairing is therefore written down, and this table's own completeness is asserted
    // below rather than assumed. The `bool` is *"this rule is emitted by the derive"*.
    const IDX_OF: &[(&str, &str, bool)] = &[
        ("bitset_storage_rejected", "IDX_BITSET_STORAGE", true),
        ("vec_field_rejected", "IDX_OPAQUE_FIELD", true),
        ("fieldless_enum_without_repr_rejected", "IDX_FIELDLESS_ENUM_WITHOUT_REPR", true),
        ("data_carrying_enum_rejected", "IDX_DATA_CARRYING_ENUM", true),
        ("union_rejected", "IDX_UNION", true),
        ("missing_default_rejected", "IDX_MISSING_DEFAULT", false),
    ];

    let src = read_code(DERIVE_SRC);
    let rows = refusals();
    assert_eq!(
        rows.len(),
        IDX_OF.len(),
        "a `REFUSALS` row was added or removed without its entry in this clause's pairing \
         table, so the new row's constant would go unchecked"
    );
    assert!(
        IDX_OF.iter().filter(|(_, _, emitted)| !*emitted).count() == 1,
        "exactly ONE row is message-only (D20's); a second exemption would be a refusal \
         nothing in the derive emits and nothing outside it pins"
    );
    // ⚠️ A clause counting `quote_spanned!` occurrences was written here and DELETED, on a
    // measurement: it reds in the green state, because this file's own rustdoc explains the
    // mechanism and spells the macro's name three times. That is the "a census clause
    // matched its own file's PROSE" defect this campaign has already met once, and a bare
    // macro name is exactly the shape that invites it. The clauses below index by
    // `REFUSALS[IDX_X].0` / `.1`, which the prose writes with a Unicode ellipsis in place of
    // a real constant and therefore cannot satisfy. Whether a site uses `quote_spanned!`
    // rather than `quote!` is not a text property at all -- it is a CARET, and C9's second
    // RED measures it where carets live, in the blessed `.stderr`.

    for row in rows {
        let (name, emitted) = IDX_OF
            .iter()
            .find(|(rule, _, _)| *rule == row.rule)
            .map(|(_, idx, emitted)| (*idx, *emitted))
            .unwrap_or_else(|| {
                panic!(
                    "row `{}` has no entry in this clause's rule -> IDX_ pairing table",
                    row.rule
                )
            });

        assert_eq!(
            src.matches(&format!("REFUSALS[{name}].0")).count(),
            1,
            "`{name}` is not name-pinned exactly once at const-eval in {DERIVE_SRC}. \
             Without `same_str(REFUSALS[{name}].0, ...)` the index and the table are two \
             lists ordered by convention, and inserting a row anywhere but the end silently \
             re-points every index after it."
        );

        if emitted {
            assert_eq!(
                src.matches(&format!("REFUSALS[{name}].1")).count(),
                1,
                "`{name}` must be read at EXACTLY ONE refusal site in {DERIVE_SRC}. Zero \
                 means the row is declared, name-pinned and never emitted from -- a refusal \
                 that does not exist, which is the dead-datum class this campaign has found \
                 five times and the exact state C9's fifth RED produces. More than one means \
                 the same rule emits from two places, and only one of them is the caret a \
                 `.stderr` pins."
            );
        } else {
            assert!(
                !src.contains(&format!("REFUSALS[{name}].1")),
                "`{name}` is marked message-only but the derive emits from it. Either the \
                 refusal became a `compile_error!` -- in which case this clause's table is \
                 stale -- or D20's trait bound has grown a second, unpinned carrier."
            );
        }
    }
}

/// **The message-only row is byte-identical to `ReflectDefault`'s `on_unimplemented`.**
///
/// D20's refusal cannot be a `compile_error!` — a proc macro cannot see trait impls — so it
/// is a trait bound wearing a named diagnostic, and its bytes live in `boyko_reflect` while
/// its row lives in `boyko_macros`. The two cannot share a const: `boyko_macros` must never
/// gain an edge to `boyko_reflect` (D17). A census clause is therefore the only thing that
/// can keep them equal, and without it the row would be a label that drifts silently away
/// from the message it claims to enumerate.
#[test]
fn the_message_only_row_matches_the_on_unimplemented_attribute() {
    let row = refusals()
        .into_iter()
        .find(|r| r.rule == MESSAGE_ONLY_ROW)
        .unwrap_or_else(|| panic!("`REFUSALS` no longer carries the `{MESSAGE_ONLY_ROW}` row"));

    let src = read_code(REFLECT_TRAIT_SRC);
    assert!(
        src.contains("#[diagnostic::on_unimplemented("),
        "{REFLECT_TRAIT_SRC} no longer carries an `on_unimplemented` attribute -- D20's \
         refusal has lost its message and this clause has no subject"
    );
    let needle = format!("message = \"{}\"", row.message);
    assert!(
        src.contains(&needle),
        "the `{MESSAGE_ONLY_ROW}` row and `ReflectDefault`'s `on_unimplemented` message \
         have drifted.\n\n  REFUSALS says: {}\n\n\
         They are two copies of one string in two crates that may never share a const \
         (D17), so nothing but this clause can hold them together.",
        row.message
    );
}

/// **The upstream pins stay OUT of the census (D34).**
///
/// `generic_component_rejected` and `repr_packed_rejected` pin refusals C9 does **not**
/// author: rustc and `#[derive(Component)]` already refuse those inputs, so the prescribed
/// red — *delete the refusal from the derive, watch the fixture compile* — is unobservable
/// for them. Counting them would make `REFUSALS` claim rules the derive does not emit, and
/// two of its rows would then be fixtures whose reds cannot fire.
#[test]
fn the_upstream_pins_are_not_counted_as_refusals() {
    let named: BTreeSet<String> = refusals().into_iter().map(|r| r.rule).collect();
    let upstream = fixture_stems(UPSTREAM_DIR);
    let leaked: Vec<&String> = upstream.intersection(&named).collect();
    assert!(
        leaked.is_empty(),
        "these upstream pins have gained a `REFUSALS` row: {leaked:?}. A row C9 does not \
         author is a fixture whose red cannot fire -- deleting C9's refusal leaves the \
         program non-compiling anyway."
    );
}
