//! GATE 3 — MALFORMED RECOVERY (Decision 6).
//!
//! A corpus of `.ui` sources with bad lines must:
//!   * NEVER panic at the file level (`parse_ui` always returns a `ParsedTree`);
//!   * record each malformed construct in `report.errors` with a `(line, col,
//!     reason)`;
//!   * PARSE THE REST — the surviving nodes/components are present in the tree.
//!
//! The error classes from the plan: bad field value, off-step indentation,
//! dedent-to-a-never-opened column, unknown component, cross-type literal (a unit
//! literal in an enum field / an enum ident in a unit field), and a duplicate
//! `#name` (demoted to anonymous, the first kept).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use boyko_ui::text::parse_ui;

/// Parses `src`, then LOWERS it through `spawn_ui_tree`, returning the report
/// accumulated during lowering (the type-directed per-field parse + the
/// closed-vocabulary dispatch run at lowering time, Decision 3/4 — so a bad field
/// value / unknown component surfaces HERE, not in the purely-syntactic
/// `parse_ui` report).
fn lower_report(src: &str) -> boyko_ui::text::UiParseReport {
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_ui::text::spawn_ui_tree;
    use std::sync::{Arc, Mutex};

    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let cell: Arc<Mutex<boyko_ui::text::UiParseReport>> =
        Arc::new(Mutex::new(boyko_ui::text::UiParseReport::default()));
    let probe = Arc::clone(&cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        // Start from the PARSE report so syntactic + lowering errors are merged.
        let mut report = owned.report.clone();
        let _ = spawn_ui_tree(&owned, &mut cmds, &mut report);
        *probe.lock().unwrap() = report;
    });
    cell.lock().unwrap().clone()
}

/// The set of node `#name`s present in a parsed tree (for "the rest survived").
fn names(tree: &boyko_ui::text::ParsedTree) -> Vec<String> {
    tree.nodes
        .iter()
        .filter_map(|n| n.name.as_ref().map(|s| s.text.clone()))
        .collect()
}

/// Asserts at least one error mentions `needle` (case-sensitive substring), and
/// returns the first matching `(line, col)`.
#[track_caller]
fn assert_error_mentions(tree: &boyko_ui::text::ParsedTree, needle: &str) -> (usize, u16) {
    let hit = tree
        .report
        .errors
        .iter()
        .find(|(_, _, r)| r.contains(needle));
    match hit {
        Some((line, col, _)) => (*line, *col),
        None => panic!(
            "expected an error mentioning {needle:?}; got errors: {:?}",
            tree.report.errors
        ),
    }
}

// ─────────────────────────── bad field value ──────────────────────────────

#[test]
fn malformed_bad_field_value_keeps_default_and_parses_rest() {
    // `width: nonsense` is not a Unit. The per-field parse is type-directed and
    // runs at LOWERING (Decision 4), so the recoverable error surfaces in the
    // lowering report, not the syntactic parse report.
    let src = "\
version=1
#a  UiLayout { layout_type: Column, width: nonsense, height: Px(24) }
#b  UiLayout { layout_type: Column, width: Px(10) }
";
    // The parse itself is purely syntactic and clean (the body is a valid span).
    let tree = parse_ui(src);
    assert!(tree.report.is_clean(), "parse is syntactic; a bad VALUE is a lowering error");
    // The bad value is caught at lowering.
    let report = lower_report(src);
    assert!(!report.is_clean(), "a bad field value is a recoverable lowering error");
    let (line, _col) = {
        let hit = report.errors.iter().find(|(_, _, r)| r.contains("width"));
        let (l, c, _) = hit.unwrap_or_else(|| panic!("expected a `width` error: {:?}", report.errors));
        (*l, *c)
    };
    assert_eq!(line, 2, "the bad-field error is attributed to line 2");
    // The rest parsed: both #a and #b exist in the tree.
    let ns = names(&tree);
    assert!(ns.contains(&"a".to_string()), "node #a survives the bad field");
    assert!(ns.contains(&"b".to_string()), "sibling #b parses after #a's bad field");
}

// ───────────────────────── off-step indentation ───────────────────────────

#[test]
fn malformed_off_step_indent_is_reported_and_skipped() {
    // The attached component is at +3 spaces (not a multiple of STEP=4).
    let src = "\
version=1
#root  UiLayout { layout_type: Column }
   UiSpacing { padding_left: Px(8) }
#sib  UiLayout { layout_type: Column }
";
    let tree = parse_ui(src);
    assert!(!tree.report.is_clean(), "off-step indent is recoverable");
    assert_error_mentions(&tree, "indentation");
    // The sibling at indent 0 still parses (the bad line was skipped, stack intact).
    let ns = names(&tree);
    assert!(ns.contains(&"root".to_string()), "#root survives");
    assert!(ns.contains(&"sib".to_string()), "#sib parses after the off-step line");
}

// ──────────────────── dedent to a never-opened column ──────────────────────

#[test]
fn malformed_dedent_to_mismatch_is_reported_and_siblings_survive() {
    // After a depth-2 child, a line dedents to indent 4 — but the depth-1 frame
    // is at indent 4 (a valid sibling level). To force a TRUE mismatch we dedent
    // to indent 4 from a depth-3 (indent 12) line where only indent 0 and 8 ever
    // opened. Line 5 sits at indent 4, a never-opened column.
    let src = "\
version=1
#root  UiLayout { layout_type: Column }
        #deep  UiLayout { layout_type: Column }
    #orphan  UiLayout { layout_type: Column }
#tail  UiLayout { layout_type: Column }
";
    let tree = parse_ui(src);
    assert!(!tree.report.is_clean(), "dedent-to-never-opened is recoverable");
    // Either the deep-indent jump or the orphan dedent is flagged; both are
    // alignment errors. The document-tail node must still parse.
    assert!(
        tree.report.errors.iter().any(|(_, _, r)| r.contains("align") || r.contains("indent")),
        "an alignment/indent error is recorded: {:?}",
        tree.report.errors
    );
    let ns = names(&tree);
    assert!(ns.contains(&"root".to_string()), "#root survives");
    assert!(ns.contains(&"tail".to_string()), "#tail at indent 0 parses after the mismatch");
}

// ────────────────────────── unknown component ─────────────────────────────

#[test]
fn malformed_unknown_component_is_reported_node_survives() {
    let src = "\
version=1
#a  UiLayout { layout_type: Column }
    Frobnicator { wat: 3 }
    UiSpacing { padding_left: Px(8) }
";
    let tree = parse_ui(src);
    // The parser accepts ANY IDENT as a component name syntactically; the unknown
    // is rejected at LOWERING/dispatch. So the parse itself is clean here, but the
    // component is recorded; assert the dispatch rejects it via spawn_ui_tree's
    // report. Drive a lowering pass to surface the unknown-component error.
    // (Parse keeps the component span; dispatch is the closed-vocabulary gate.)
    let comp_present = tree.nodes.iter().any(|n| n.components.iter().any(|c| c.name == "Frobnicator"));
    assert!(comp_present, "the parser captured the unknown component span");

    // Lower it to exercise the dispatch rejection (closed-match -> recoverable).
    use boyko_ecs::ecs::core::component::component::Component;
    use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_ui::text::spawn_ui_tree;
    use std::sync::{Arc, Mutex};
    let mut world = EcsMaster::new();
    let report_cell: Arc<Mutex<boyko_ui::text::UiParseReport>> =
        Arc::new(Mutex::new(boyko_ui::text::UiParseReport::default()));
    let probe = Arc::clone(&report_cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let _ = spawn_ui_tree(&owned, &mut cmds, &mut report);
        *probe.lock().unwrap() = report;
    });
    let report = report_cell.lock().unwrap().clone();
    assert!(
        report.errors.iter().any(|(_, _, r)| r.contains("unknown component")),
        "dispatch records an unknown-component error: {:?}",
        report.errors
    );
    // The node and its valid UiSpacing survived lowering.
    assert_eq!(world.query_entities(&[boyko_ui::components::UiLayout::component_id()]).len(), 1, "node #a spawned despite the unknown component");
}

// ─────────────────────────── cross-type literal ───────────────────────────

#[test]
fn malformed_cross_type_literal_is_reported() {
    // `Stretch` in an AlignCross field is VALID (AlignCross::Stretch). But a unit
    // call `Px(5)` in an AlignCross field is a cross-type literal -> recoverable
    // (caught by the type-directed leaf parser at lowering, Decision 4).
    let src = "\
version=1
#a  UiLayout { layout_type: Column }
    UiAlign { cross: Px(5) }
";
    let report = lower_report(src);
    assert!(!report.is_clean(), "a unit literal in an enum field is recoverable");
    assert!(
        report.errors.iter().any(|(_, _, r)| r.contains("cross")),
        "the cross-field error is recorded: {:?}",
        report.errors
    );

    // Inverse: a bare enum ident in a Unit field (`width: Center`) is also wrong.
    let src2 = "\
version=1
#b  UiLayout { layout_type: Column, width: Center }
";
    let report2 = lower_report(src2);
    assert!(!report2.is_clean(), "an enum ident in a Unit field is recoverable");
    assert!(
        report2.errors.iter().any(|(_, _, r)| r.contains("width")),
        "the width-field error is recorded: {:?}",
        report2.errors
    );
}

// ───────────────────────────── duplicate #name ────────────────────────────

#[test]
fn malformed_duplicate_name_demotes_second_keeps_first() {
    let src = "\
version=1
#dup  UiLayout { layout_type: Column, width: Px(10) }
#dup  UiLayout { layout_type: Column, width: Px(20) }
";
    let tree = parse_ui(src);
    assert!(!tree.report.is_clean(), "a duplicate #name is recoverable");
    let (line, _col) = assert_error_mentions(&tree, "duplicate");
    assert_eq!(line, 3, "the duplicate is flagged at its second occurrence (line 3)");

    // Exactly ONE node keeps the name `dup`; the second is demoted to anonymous.
    let dup_count = tree
        .nodes
        .iter()
        .filter(|n| n.name.as_ref().map(|s| s.text == "dup").unwrap_or(false))
        .count();
    assert_eq!(dup_count, 1, "only the FIRST #dup keeps the name");
    // Both nodes still exist (two roots), the second just unnamed.
    assert_eq!(tree.roots.len(), 2, "both nodes parse; the duplicate is kept anonymous");
}

// ─────────────────── over-CAP name demotion (Decision 6) ───────────────────

#[test]
fn malformed_over_cap_name_demotes_to_anonymous() {
    let long = "x".repeat(80); // > UiName::CAP (60)
    let src = format!(
        "version=1\n#{long}  UiLayout {{ layout_type: Column }}\n#ok  UiLayout {{ layout_type: Column }}\n"
    );
    let tree = parse_ui(&src);
    assert!(!tree.report.is_clean(), "an over-CAP name is recoverable");
    assert_error_mentions(&tree, "exceeds");
    // The node still parses (anonymous); the next named node is fine.
    assert_eq!(tree.roots.len(), 2, "both nodes parse");
    assert!(names(&tree).contains(&"ok".to_string()), "the next #name parses");
}

// ────────────── invalid version value is recoverable (Decision 6) ──────────

#[test]
fn malformed_invalid_version_value_keeps_default() {
    let src = "\
version=abc
#a  UiLayout { layout_type: Column }
";
    let tree = parse_ui(src);
    assert!(!tree.report.is_clean(), "a non-numeric version is recoverable");
    assert_error_mentions(&tree, "version");
    assert!(names(&tree).contains(&"a".to_string()), "the node after a bad version parses");
}

// ─────────────────────── malformed component span ──────────────────────────

#[test]
fn malformed_unterminated_brace_is_reported() {
    let src = "\
version=1
#a  UiLayout { layout_type: Column
#b  UiLayout { layout_type: Column }
";
    let tree = parse_ui(src);
    assert!(!tree.report.is_clean(), "an unterminated brace is recoverable");
    assert!(
        tree.report.errors.iter().any(|(_, _, r)| r.contains("malformed component") || r.contains("brace")),
        "a malformed-component error is recorded: {:?}",
        tree.report.errors
    );
}

// ──────────────── never panics over a stress corpus (fuzz-lite) ────────────

#[test]
fn malformed_stress_corpus_never_panics() {
    // A grab-bag of adversarial lines: empty names, tabs, deep indents, junk,
    // partial braces, unicode, all interleaved. The ONLY contract is "no panic +
    // returns a tree".
    let corpus = [
        "",
        "version=",
        "version=1\nversion=2",
        "#",
        "#   UiLayout { }",
        "\t\t#tab UiLayout { width: Px(1) }",
        "#a UiLayout { width: Px( }",
        "#b ) ) ) {{{ }}}",
        "#c UiLayout { width: Px(1), , , height: }",
        "    #orphan UiLayout {}",
        "#d UiLayout { layout_type: Column }\n                    #deep UiLayout {}",
        "#e StackIndex(notanumber)",
        "#f UiRoot { has: fields }",
        "// just a comment\n#g UiLayout { width: Px(1) } // trailing",
        "#h UiLayout { width: \"quoted // not a comment\" }",
        "#name_with_emoji_🎮 UiLayout {}",
    ];
    for (i, src) in corpus.iter().enumerate() {
        // The contract: this returns without unwinding.
        let tree = parse_ui(src);
        // A tree is always produced; errors may or may not be present.
        let _ = tree.is_empty();
        let _ = tree.report.is_clean();
        // Sanity: node count is finite and the roots index into nodes.
        for &r in &tree.roots {
            assert!(r < tree.nodes.len(), "corpus[{i}]: root index in bounds");
        }
    }
}
