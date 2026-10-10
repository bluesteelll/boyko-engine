//! GATE 4 — LLM / hand-authored `.ui` corpus parses AND lowers clean.
//!
//! A set of representative, realistic `.ui` documents an author (or an LLM) would
//! write: a dialog, a HUD bar, a settings panel, a list, an overlay. Each MUST
//! parse with zero errors AND lower with zero errors (the value grammar is
//! exercised by lowering), and produce the expected node count + named structure.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod p3_common;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use std::sync::{Arc, Mutex};

use boyko_ui::components::UiLayout;
use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport};

/// Parses + lowers `src`, asserting BOTH passes are clean, and returns
/// `(node_count_in_tree, spawned_entity_count)`.
#[track_caller]
fn parse_lower_clean(name: &str, src: &str) -> (usize, usize) {
    let tree = parse_ui(src);
    assert!(
        tree.report.is_clean(),
        "corpus `{name}`: parse must be clean, got errors: {:?}\nwarnings: {:?}",
        tree.report.errors,
        tree.report.warnings
    );
    let node_count = tree.nodes.len();

    let mut world = EcsMaster::new();
    let cell: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let probe = Arc::clone(&cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let _ = spawn_ui_tree(&owned, &mut cmds, &mut report);
        *probe.lock().unwrap() = report;
    });
    let report = cell.lock().unwrap().clone();
    assert!(
        report.is_clean(),
        "corpus `{name}`: lowering must be clean, got errors: {:?}",
        report.errors
    );
    let spawned = world.query_entities(&[UiLayout::component_id()]).len();
    (node_count, spawned)
}

#[test]
fn corpus_dialog() {
    let src = "\
// A modal confirm dialog.
version=1
#dialog_root  UiLayout { layout_type: Column, width: Px(360), height: Px(200) }
    UiRoot
    UiSpacing { padding_left: Px(16), padding_right: Px(16), padding_top: Px(16), padding_bottom: Px(16) }
    StackIndex(100)
    #title  UiLayout { layout_type: Row, width: Stretch(1), height: Px(28) }
        ContentSize { width: 0, height: 28 }
    #message  UiLayout { layout_type: Column, width: Stretch(1), height: Stretch(1) }
    #buttons  UiLayout { layout_type: Row, width: Stretch(1), height: Px(36) }
        UiSpacing { column_gap: Px(8) }
        UiAlign { main: End }
        #ok  UiLayout { layout_type: Column, width: Px(96), height: Px(32) }
        #cancel  UiLayout { layout_type: Column, width: Px(96), height: Px(32) }
";
    // root + title + message + buttons + ok + cancel = 6 nodes (the `ContentSize`
    // / `UiSpacing` / `UiAlign` lines are ATTACHED components at +STEP, not nodes).
    let (nodes, spawned) = parse_lower_clean("dialog", src);
    assert_eq!(nodes, 6, "dialog has 6 nodes");
    assert_eq!(spawned, 6, "dialog spawned 6 entities");
}

#[test]
fn corpus_hud_bar() {
    let src = "\
version=1
#hud  UiLayout { layout_type: Row, position_type: Absolute, width: Stretch(1), height: Px(48) }
    UiRoot
    UiAbsolute { left: Px(0), top: Px(0) }
    UiSpacing { padding_left: Px(12), column_gap: Px(24) }
    UiAlign { main: SpaceBetween, cross: Center }
    #health  UiLayout { layout_type: Row, width: Px(160), height: Px(24) }
    #score  UiLayout { layout_type: Row, width: Px(120), height: Px(24) }
    #minimap  UiLayout { layout_type: Overlay, width: Px(160), height: Px(160) }
";
    let (nodes, spawned) = parse_lower_clean("hud_bar", src);
    assert_eq!(nodes, 4, "hud has 4 nodes");
    assert_eq!(spawned, 4, "hud spawned 4 entities");
}

#[test]
fn corpus_settings_panel() {
    let src = "\
version=1
#settings  UiLayout { layout_type: Column, width: Px(480), height: Stretch(1) }
    UiRoot
    UiSpacing { padding_top: Px(8), row_gap: Px(6) }
    #row_volume  UiLayout { layout_type: Row, width: Stretch(1), height: Px(32) }
        UiAlign { main: SpaceBetween, cross: Center }
        #label_volume  UiLayout { layout_type: Column, width: Px(200), height: Px(24) }
        #slider_volume  UiLayout { layout_type: Row, width: Px(240), height: Px(24) }
    #row_fullscreen  UiLayout { layout_type: Row, width: Stretch(1), height: Px(32) }
        UiAlign { main: SpaceBetween, cross: Center }
        #label_fullscreen  UiLayout { layout_type: Column, width: Px(200), height: Px(24) }
        #toggle_fullscreen  UiLayout { layout_type: Row, width: Px(48), height: Px(24) }
";
    let (nodes, spawned) = parse_lower_clean("settings_panel", src);
    assert_eq!(nodes, 7, "settings has 7 nodes");
    assert_eq!(spawned, 7, "settings spawned 7 entities");
}

#[test]
fn corpus_list_with_named_items() {
    // A realistic list: the root's attached components (UiRoot/UiSpacing) at +STEP,
    // then NAMED child items — a `#`-head at +STEP is a CHILD (an IDENT-head at
    // +STEP would instead ATTACH, Decision 1).
    let src = "\
version=1
#list  UiLayout { layout_type: Column, width: Px(300), height: Stretch(1) }
    UiRoot
    UiSpacing { row_gap: Px(2) }
    #item0  UiLayout { layout_type: Row, width: Stretch(1), height: Px(28) }
    #item1  UiLayout { layout_type: Row, width: Stretch(1), height: Px(28) }
    #item2  UiLayout { layout_type: Row, width: Stretch(1), height: Px(28) }
";
    let (nodes, spawned) = parse_lower_clean("list", src);
    assert_eq!(nodes, 4, "list has 4 nodes (1 root + 3 items)");
    assert_eq!(spawned, 4, "list spawned 4 entities");

    // No warnings (no anonymous state-bearing nodes).
    let tree = parse_ui(src);
    assert!(
        tree.report.warnings.is_empty(),
        "named list items produce no warning: {:?}",
        tree.report.warnings
    );
    // The three items are children of the root in declaration order.
    let root = &tree.nodes[tree.roots[0]];
    assert_eq!(root.children.len(), 3, "root has 3 item children");
}

#[test]
fn corpus_anonymous_child_node_grammar() {
    // Documents the actual anonymous-child grammar: a bare-component line at the
    // CHILD level (sibling of a named child) opens an anonymous node. Here #named
    // establishes the depth-4 child level; the bare `UiLayout` at depth 4 that
    // FOLLOWS it (rel==0 vs the #named frame) is an anonymous SIBLING child of the
    // root, not an attached component of #named.
    let src = "\
version=1
#root  UiLayout { layout_type: Column }
    #named  UiLayout { layout_type: Row, height: Px(10) }
    UiLayout { layout_type: Row, height: Px(20) }
";
    let tree = parse_ui(src);
    assert!(tree.report.is_clean(), "anonymous-sibling grammar parses clean: {:?}", tree.report.errors);
    assert_eq!(tree.nodes.len(), 3, "root + #named + 1 anonymous child = 3 nodes");
    let root = &tree.nodes[tree.roots[0]];
    assert_eq!(root.children.len(), 2, "root has #named and the anonymous node as children");
    // The second child is anonymous.
    let anon = &tree.nodes[root.children[1]];
    assert!(anon.name.is_none(), "the trailing bare-component node is anonymous");
    assert_eq!(anon.sibling_ordinal, 1, "the anonymous child has declaration ordinal 1");
}

#[test]
fn corpus_overlay_stack() {
    let src = "\
version=1
#screen  UiLayout { layout_type: Overlay, width: Stretch(1), height: Stretch(1) }
    UiRoot
    #background  UiLayout { layout_type: Column, width: Stretch(1), height: Stretch(1) }
        StackIndex(0)
    #content  UiLayout { layout_type: Column, width: Stretch(1), height: Stretch(1) }
        StackIndex(1)
    #modal  UiLayout { layout_type: Column, width: Px(400), height: Px(300) }
        StackIndex(2)
        UiAlign { main: Center, cross: Center }
";
    let (nodes, spawned) = parse_lower_clean("overlay_stack", src);
    assert_eq!(nodes, 4, "overlay has 4 nodes");
    assert_eq!(spawned, 4, "overlay spawned 4 entities");
}

#[test]
fn corpus_minimal_single_node() {
    let src = "\
version=1
#only  UiLayout { layout_type: Column }
";
    let (nodes, spawned) = parse_lower_clean("minimal", src);
    assert_eq!(nodes, 1, "minimal has 1 node");
    assert_eq!(spawned, 1, "minimal spawned 1 entity");
}

#[test]
fn corpus_comments_and_blank_lines() {
    // Comments (`//`) and blank lines must be ignored without perturbing the
    // indentation tree.
    let src = "\
// top-level comment

version=1

// the root
#root  UiLayout { layout_type: Column, width: Px(100) }   // trailing comment

    // a child follows
    #child  UiLayout { layout_type: Row, width: Px(50) }

";
    let (nodes, spawned) = parse_lower_clean("comments", src);
    assert_eq!(nodes, 2, "comments-doc has 2 nodes (root + child)");
    assert_eq!(spawned, 2, "comments-doc spawned 2 entities");
}
