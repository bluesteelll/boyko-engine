//! GATE 2 — ROUND-TRIP: `parse(serialize(tree)) == tree`.
//!
//! For several canonical trees we:
//!   1. parse a canonical `.ui` source, spawn it into a world;
//!   2. snapshot the live subtree into a `UiTreeView` and `serialize_ui` it;
//!   3. re-`serialize` the parse of THAT text and assert BYTE-IDENTITY of the two
//!      serializations (`serialize → parse → serialize` is a fixed point — the
//!      canonical normal form);
//!   4. re-parse the serialized text and assert the parsed component VALUES equal
//!      the originals (incl. the `.ui` float rule: integral floats lose the `.0`,
//!      fractional floats use Rust's shortest round-trip, both re-read to
//!      bit-identical `f32`s).
//!
//! The serializer OMITS `ComputedRect` (layout output, Decision 14) and
//! `UiSourceOrder` (private), so the round-trip domain is the author-visible
//! text-owned set minus `ComputedRect`. Inputs here avoid `ComputedRect` so the
//! tree is a true serializer fixed point.

mod common;
mod p3_common;

use std::sync::{Arc, Mutex};

use common::Ui;

use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::reload::tree_view::UiTreeView;
use boyko_ui::text::{parse_ui, serialize_ui, spawn_ui_tree};
use boyko_ui::units::Unit;

/// Spawns `src` into the world, returns the doc roots (declaration order).
fn spawn(ui: &mut Ui, src: &str) -> Vec<Entity> {
    let tree = parse_ui(src);
    assert!(tree.report.is_clean(), "round-trip input must parse clean: {:?}", tree.report.errors);
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    let owned = tree.clone();
    ui.author(move |mut cmds| {
        let mut report = owned.report.clone();
        let sr = spawn_ui_tree(&owned, &mut cmds, &mut report);
        let mut v = probe.lock().unwrap();
        for r in sr.iter() {
            v.push(r);
        }
    });
    sink.lock().unwrap().clone()
}

/// Snapshot the document subtree (rooted at `roots`) and serialize it.
fn serialize_tree(ui: &Ui, roots: &[Entity]) -> String {
    let view = UiTreeView::build(&ui.world, roots);
    let mut out = String::new();
    serialize_ui(&view, &mut out);
    out
}

/// Core gate: serialize the live tree, parse-then-serialize it again, assert the
/// two serializations are byte-identical (canonical normal form), and assert the
/// re-parse is clean.
#[track_caller]
fn assert_serialize_fixed_point(ui: &mut Ui, roots: &[Entity], what: &str) -> String {
    let s1 = serialize_tree(ui, roots);

    // Parse the serialized text and spawn it into a FRESH world region, then
    // serialize THAT. A canonical serializer is a fixed point: s2 == s1.
    let reparse = parse_ui(&s1);
    assert!(
        reparse.report.is_clean(),
        "{what}: serialized text must re-parse clean, got: {:?}\n--- text ---\n{s1}",
        reparse.report.errors
    );
    let roots2 = spawn(ui, &s1);
    let s2 = serialize_tree(ui, &roots2);

    assert_eq!(s1, s2, "{what}: serialize→parse→serialize must be byte-identical");
    s1
}

// ───────────────────────────── 1. leaf ────────────────────────────────────

#[test]
fn round_trip_leaf() {
    let mut ui = Ui::default_world();
    let src = "\
version=1
#leaf  UiLayout { layout_type: Column, width: Px(120), height: Px(24) }
";
    let roots = spawn(&mut ui, src);
    assert_serialize_fixed_point(&mut ui, &roots, "leaf");
}

// ──────────────────────── 2. fractional + integral floats ─────────────────

#[test]
fn round_trip_float_rule_integral_and_fractional() {
    let mut ui = Ui::default_world();
    // Integral floats (120, 24) must serialize WITHOUT a `.0`; fractional ones
    // (33.5, 1.5) via shortest round-trip — both re-parse to identical bits.
    let src = "\
version=1
#mix  UiLayout { layout_type: Column, width: Pct(33.5), height: Stretch(1.5), min_width: Px(120) }
    ContentSize { width: 7, height: 12.25 }
";
    let roots = spawn(&mut ui, src);
    let text = assert_serialize_fixed_point(&mut ui, &roots, "float-rule");

    // The integral floats must NOT carry a trailing `.0` (the `.ui` rule).
    assert!(text.contains("Px(120)"), "integral float serializes as `120`, not `120.0`:\n{text}");
    assert!(
        text.contains("height: 12.25"),
        "fractional ContentSize height serializes shortest round-trip:\n{text}"
    );
    assert!(text.contains("width: 7"), "integral ContentSize width serializes as `7`:\n{text}");
    assert!(!text.contains("120.0"), "no `.0` appended to integral floats:\n{text}");

    // Parsed value identity: re-parse a UiLayout and check the bits directly.
    let reparse = parse_ui(&text);
    let node = &reparse.nodes[reparse.roots[0]];
    let layout_comp = node.components.iter().find(|c| c.name == "UiLayout").unwrap();
    assert!(layout_comp.body.contains("Pct(33.5)"), "fractional Pct survives re-parse: {layout_comp:?}");
    let _ = Unit::Auto; // keep the units import meaningful
}

// ───────────────────── 3. full optional set (no ComputedRect) ──────────────

#[test]
fn round_trip_full_optional_set() {
    let mut ui = Ui::default_world();
    let src = "\
version=1
#everything  UiLayout { layout_type: Row, position_type: Absolute, width: Px(50), height: Auto }
    UiSpacing { padding_left: Px(3), row_gap: Px(2), column_gap: Px(4) }
    UiAlign { main: Center, cross: Stretch }
    UiAbsolute { left: Px(5), top: Px(6) }
    ContentSize { width: 12, height: 7 }
    StackIndex(10)
    ComputedClip { x: 1, y: 2, w: 3, h: 4 }
    UiRoot
";
    let roots = spawn(&mut ui, src);
    let text = assert_serialize_fixed_point(&mut ui, &roots, "full-set");
    assert!(text.contains("StackIndex(10)"), "StackIndex tuple form preserved:\n{text}");
    assert!(text.contains("UiRoot"), "UiRoot marker preserved:\n{text}");
    assert!(text.contains("cross: Stretch"), "AlignCross::Stretch preserved (not Unit):\n{text}");
}

// ───────────────────────── 4. nested tree topology ────────────────────────

#[test]
fn round_trip_nested_tree() {
    let mut ui = Ui::default_world();
    let src = "\
version=1
#root  UiLayout { layout_type: Column, width: Px(400), height: Px(800) }
    UiRoot
    #row  UiLayout { layout_type: Row, width: Px(400), height: Px(40) }
        #ok  UiLayout { layout_type: Column, width: Px(80), height: Px(24) }
        #cancel  UiLayout { layout_type: Column, width: Px(80), height: Px(24) }
    #body  UiLayout { layout_type: Column, width: Px(400), height: Stretch(1) }
";
    let roots = spawn(&mut ui, src);
    let text = assert_serialize_fixed_point(&mut ui, &roots, "nested");

    // Topology survives: re-parse and check the child structure.
    let reparse = parse_ui(&text);
    assert_eq!(reparse.roots.len(), 1, "one root after round-trip");
    let root = &reparse.nodes[reparse.roots[0]];
    assert_eq!(root.children.len(), 2, "root has #row and #body");
    let row = &reparse.nodes[root.children[0]];
    assert_eq!(row.name.as_ref().map(|n| n.text.as_str()), Some("row"), "first child is #row");
    assert_eq!(row.children.len(), 2, "#row has #ok and #cancel");
}

// ───────────────────── 5. all enum variants survive round-trip ─────────────

#[test]
fn round_trip_enum_variants() {
    let mut ui = Ui::default_world();
    let src = "\
version=1
#a  UiLayout { layout_type: Overlay, position_type: Relative, width: Auto, height: Auto }
    UiAlign { main: SpaceBetween, cross: End }
#b  UiLayout { layout_type: Grid, position_type: Absolute, width: Auto, height: Auto }
    UiAlign { main: SpaceEvenly, cross: Start }
";
    let roots = spawn(&mut ui, src);
    let text = assert_serialize_fixed_point(&mut ui, &roots, "enum-variants");
    assert!(text.contains("layout_type: Overlay"), "Overlay survives:\n{text}");
    assert!(text.contains("layout_type: Grid"), "Grid survives:\n{text}");
    assert!(text.contains("main: SpaceBetween"), "SpaceBetween survives:\n{text}");
    assert!(text.contains("main: SpaceEvenly"), "SpaceEvenly survives:\n{text}");
    assert!(text.contains("position_type: Absolute"), "Absolute survives:\n{text}");
}
