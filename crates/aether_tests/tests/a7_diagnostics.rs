//! Rung A7's trybuild goldens — the DX-hardening half of the §7.5 pyramid.
//!
//! Four cases, one per A7 contract item:
//!
//! * `version_header_unknown` / `version_header_out_of_place` — §6.3's `aether v1;` gate, refused
//!   on the version token and on the position;
//! * `scene_at_bare_path_struct_literal` — the `at BARE_PATH { … }` trap, named in the message it
//!   used to contradict;
//! * `material_duplicate_key` — a duplicate key with BOTH spans;
//! * `recovery_one_typo_costs_one_error` — §8 R3's rust-analyzer resilience case, whose contract
//!   is the SIZE of the `.stderr`: one fault, one error, every sibling still resolvable;
//! * `recovery_broken_plugin_keeps_the_block` — the same contract at the whole-block RULES: a
//!   half-typed `plugin` holds the plugin slot, so no sibling clause reports a second fault;
//! * `recovery_duplicate_name_with_a_broken_twin` / `..._type_name_...` — a duplicate where one of
//!   the two names is unreadable, on both halves of §4's rule (Aether owns the fn half with two
//!   spans; rustc owns the type half, and the stub's span policy keeps ITS labels on user tokens
//!   too).
//!
//! # Why these are goldens and not unit tests (the R2 half)
//!
//! `aether-lang`'s unit tests pin each message's TEXT. A `.stderr` pins the other half of §7.2's
//! span rule — the LINE AND COLUMN the error lands on, in a real downstream compilation. A
//! message that is right in a unit test but anchored at `Span::call_site()` passes there and
//! fails here, and R2 (span degradation from stringify/re-parse round-trips) is precisely the
//! regression that would show up as a column moving to the `aether!` token and nowhere else.
//!
//! Blessing discipline (the repo's trybuild rule): a `.stderr` is re-blessed ONLY after verifying
//! the error KIND and the caret's position are what the case is about — the
//! `token_use_after_submit_rejected` lesson (87 commits red because a line moved and nobody
//! re-blessed one file).

use std::fs;
use std::path::Path;

#[test]
fn a7_diagnostics_land_on_the_users_tokens() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/version_header_unknown.rs");
    t.compile_fail("tests/ui/version_header_out_of_place.rs");
    t.compile_fail("tests/ui/scene_at_bare_path_struct_literal.rs");
    t.compile_fail("tests/ui/material_duplicate_key.rs");
    t.compile_fail("tests/ui/recovery_one_typo_costs_one_error.rs");
    t.compile_fail("tests/ui/recovery_broken_plugin_keeps_the_block.rs");
    t.compile_fail("tests/ui/recovery_duplicate_name_with_a_broken_twin.rs");
    t.compile_fail("tests/ui/recovery_duplicate_type_name_with_a_broken_twin.rs");
}

/// R2's sweep over the WHOLE golden corpus, as a mechanical check rather than a habit.
///
/// Three properties, each of which has failed silently somewhere in this repo's history:
///
/// 1. **Every `tests/ui/*.rs` is registered** in some `a*_diagnostics.rs` `compile_fail` list. An
///    unregistered fixture is a golden nobody runs — it cannot go red, so its `.stderr` records
///    whatever the compiler said the day it was written, forever. (The dead-datum class: a datum
///    nobody re-derives, which the first "fix" re-blesses.)
/// 2. **Every fixture has a `.stderr` that pins at least one `line:column`.** That is what R2 is
///    about: the message is the half a unit test can hold, the caret position is the half only a
///    golden can.
/// 3. **No label sits on the `aether! {` line — primary OR secondary.** This is the SIGNATURE of
///    span degradation: a diagnostic that lost the user's span falls back to the macro token,
///    which is technically a location and practically useless in a forty-line block. A
///    stringify/re-parse round trip introduced anywhere in `aether_lang` collapses spans exactly
///    this way, and it would pass every message assertion in the crate's unit tests.
///
///    The SECONDARY half of that check is not decoration. As first written this sweep read only
///    `-->` lines (the primary span), and the very next golden added to the corpus —
///    `recovery_duplicate_type_name_with_a_broken_twin` — carried
///    `17 | aether! {  | ------- previous definition of the type `Foo` here`: a label on the macro
///    line, invisible to a detector whose doc claimed it caught lost spans. The cause was real
///    (type-producing items were emitted with `quote!`, i.e. at `Span::call_site()`, against
///    §7.2(3)); the fix was in `expand.rs`, and this half of the check is what would have found
///    it. Gutter lines (`NN | source`) name every line a diagnostic points at, whichever kind of
///    label put it there.
#[test]
fn every_ui_golden_is_registered_and_pins_a_span_off_the_macro_token() {
    let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");

    let mut registered: Vec<String> = Vec::new();
    for entry in fs::read_dir(&tests_dir).expect("invariant: the tests dir exists") {
        let path = entry.expect("invariant: readable dir entry").path();
        let is_suite = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_diagnostics.rs"));
        if !is_suite {
            continue;
        }
        let src = fs::read_to_string(&path).expect("invariant: readable suite file");
        for piece in src.split("compile_fail(\"").skip(1) {
            let Some(end) = piece.find('"') else { continue };
            registered.push(piece[..end].to_string());
        }
    }
    assert!(registered.len() >= 30, "the suite list itself looks truncated: {registered:?}");

    let ui_dir = tests_dir.join("ui");
    let mut checked = 0usize;
    for entry in fs::read_dir(&ui_dir).expect("invariant: the ui dir exists") {
        let path = entry.expect("invariant: readable dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path.file_stem().and_then(|n| n.to_str()).expect("utf-8 fixture name");
        let rel = format!("tests/ui/{name}.rs");
        assert!(
            registered.contains(&rel),
            "`{rel}` is a compile-fail fixture no `a*_diagnostics.rs` runs — register it or delete it"
        );

        let fixture = fs::read_to_string(&path).expect("invariant: readable fixture");
        let macro_line = fixture
            .lines()
            .position(|l| l.trim() == "aether! {")
            .map(|i| i + 1)
            .expect("every fixture opens its block on a line of its own");

        let stderr = fs::read_to_string(ui_dir.join(format!("{name}.stderr")))
            .unwrap_or_else(|_| panic!("`{name}.rs` has no pinned `.stderr`"));
        let mut spans = 0usize;
        for line in stderr.lines() {
            // Primary spans: `--> tests/ui/<name>.rs:LINE:COL`.
            if let Some(rest) = line.trim().strip_prefix("--> ")
                && let Some(loc) = rest.strip_prefix(&format!("{rel}:"))
            {
                let mut parts = loc.split(':');
                let l: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or_default();
                let c: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or_default();
                assert!(l > 0 && c > 0, "`{name}`: unparsed span `{loc}`");
                assert_ne!(
                    l, macro_line,
                    "`{name}`: a primary span landed on the `aether! {{` line — that is what a lost span looks like (§7.2(1): verbatim tokens, never stringify/re-parse)"
                );
                spans += 1;
                continue;
            }
            // Every OTHER label, primary or secondary, quotes its source line in the gutter:
            // `NN | <source>`. Any of them on the macro line is the same degradation.
            if let Some(l) = gutter_line(line) {
                assert_ne!(
                    l, macro_line,
                    "`{name}`: a label (`{}`) quotes the `aether! {{` line — a secondary label at `Span::call_site()` is a lost span too (§7.2(3): items exist because of a user name and are spanned at it)",
                    line.trim()
                );
            }
        }
        assert!(spans > 0, "`{name}.stderr` pins no `line:column` at all");
        checked += 1;
    }
    assert!(checked >= 30, "swept only {checked} fixtures — the corpus should be larger");
}

/// The source line a rustc snippet line quotes, if it quotes one.
///
/// rustc's snippet gutter is `<line> | <source>` (with a `/` or `|` column for multi-line spans:
/// `21 | / aether! {`). Everything else — the `   |` rails, the `...` elision, `= note:` — has no
/// number to the left of the bar and answers `None`.
fn gutter_line(line: &str) -> Option<usize> {
    let (left, _) = line.split_once('|')?;
    left.trim().parse().ok()
}
