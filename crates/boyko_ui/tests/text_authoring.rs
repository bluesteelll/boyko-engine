//! GUI P5b — the AUTHORING gate (the plan's gate 5 / T6): a `UiText` style component is
//! spawnable via BOTH authoring surfaces — the `ui!` proc-macro AND the `.ui` text format
//! — and the two paths produce an IDENTICAL `UiText` value on an identical archetype.
//!
//! `UiText` is a normal `#[derive(Component)]` POD, so the `ui!` macro lowers it through
//! its generic component-insert arm (`boyko_macros` `lower_node` emits each non-bundle
//! component literal as a chained `insert`), and the `.ui` loader parses it through the
//! `parse_ui_text` dispatch arm (`text/dispatch.rs`). This test PROVES both arms exist
//! and AGREE (no DSL/macro drift on the new component), the same `ui! ≡ .ui` equivalence
//! the P3 suite established for the layout vocabulary, now extended to `UiText`.
//!
//! These run on the shared `Ui` harness (Commands-driven, one apply window), no GPU.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Mutex};

use common::Ui;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::components::UiLayout;
use boyko_ui::prelude::ui;
use boyko_ui::text::{parse_ui, spawn_ui_tree, FontId, TextAlign, UiText};
use boyko_ui::units::{LayoutType, Unit};

/// The `UiText` style this gate authors both ways. Non-default in every field so a
/// dropped/defaulted field on either arm is caught: orange-ish color, 22 px, font 0
/// (the single resident font, Decision T4-E), centre-aligned.
fn expected_text() -> UiText {
    UiText { color: 0xFF8800FF, size_px: 22.0, font: FontId(0), align: TextAlign::Center, _pad: 0 }
}

/// Spawns one `UiLayout + UiText` node via the `ui!` macro, returning its handle.
fn spawn_via_macro(ui: &mut Ui) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiLayout { layout_type: LayoutType::Column, width: Unit::Auto, height: Unit::Auto, ..UiLayout::default() },
            UiText {
                color: 0xFF8800FF,
                size_px: 22.0,
                font: ::boyko_ui::text::FontId(0),
                align: ::boyko_ui::text::TextAlign::Center,
                _pad: 0
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    sink.lock().unwrap().expect("macro-spawned UiText node")
}

/// Spawns one `UiLayout + UiText` node from a `.ui` source via the runtime loader,
/// returning its handle. The `.ui` grammar parses `UiText` through `parse_ui_text`.
fn spawn_via_dotui(ui: &mut Ui) -> Entity {
    // One `.ui` NODE: the `UiLayout` header line + the `UiText` component INDENTED under
    // it (the `.ui` grammar groups a node's extra components by indentation — a column-0
    // sibling line would be a separate node). The `u32` leaf parser is decimal-only
    // (`u32::from_str`, no `0x` prefix), so the color is the decimal of `0xFF8800FF`
    // (= 4287103231) — the SAME numeric value the `ui!` arm authors as a hex literal.
    const SRC: &str = "\
UiLayout { layout_type: Column, width: Auto, height: Auto }
    UiText { color: 4287103231, size_px: 22.0, font: 0, align: Center }
";
    let tree = parse_ui(SRC);
    assert!(
        tree.report.is_clean(),
        ".ui source must parse cleanly: {:?}",
        tree.report.errors
    );

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let owned = tree.clone();
    ui.author(move |mut cmds| {
        let mut report = owned.report.clone();
        let roots = spawn_ui_tree(&owned, &mut cmds, &mut report);
        assert!(report.is_clean(), "lowering must be clean: {:?}", report.errors);
        *probe.lock().unwrap() = roots.iter().next();
    });
    sink.lock().unwrap().expect(".ui-spawned UiText node")
}

/// Gate 5a — the `ui!` macro spawns a `UiText` carrying every authored field verbatim.
#[test]
fn ui_macro_spawns_uitext_with_authored_fields() {
    let mut ui = Ui::default_world();
    let e = spawn_via_macro(&mut ui);

    let got = ui.world.get_component::<UiText>(e).copied().expect("node carries UiText");
    assert_eq!(got, expected_text(), "ui! macro UiText fields must match the authored literal");
}

/// Gate 5b — the `.ui` loader spawns a `UiText` carrying every authored field verbatim.
#[test]
fn dotui_loader_spawns_uitext_with_authored_fields() {
    let mut ui = Ui::default_world();
    let e = spawn_via_dotui(&mut ui);

    let got = ui.world.get_component::<UiText>(e).copied().expect("node carries UiText");
    assert_eq!(got, expected_text(), ".ui loader UiText fields must match the authored source");
}

/// Gate 5c — `ui!` ≡ `.ui`: the macro arm and the text-format arm produce the SAME
/// `UiText` value AND the SAME archetype (the new component does not drift between the
/// two authoring surfaces — the core P3/P5b equivalence guarantee).
#[test]
fn ui_macro_and_dotui_agree_on_uitext() {
    let mut ui = Ui::default_world();
    let m = spawn_via_macro(&mut ui);
    let d = spawn_via_dotui(&mut ui);

    let mt = ui.world.get_component::<UiText>(m).copied().expect("macro node UiText");
    let dt = ui.world.get_component::<UiText>(d).copied().expect("dotui node UiText");
    assert_eq!(mt, dt, "ui! and .ui must produce an identical UiText value");

    // Both carry UiText + UiLayout; the archetypes (component sets) coincide. (.ui adds a
    // private UiSourceOrder stamp that is gate-excluded by the P3 equivalence contract,
    // so compare on the AUTHORED component set rather than raw archetype id.)
    assert!(ui.world.has_component(m, UiText::component_id()), "macro node has UiText");
    assert!(ui.world.has_component(d, UiText::component_id()), "dotui node has UiText");
    assert!(ui.world.has_component(m, UiLayout::component_id()), "macro node has UiLayout");
    assert!(ui.world.has_component(d, UiLayout::component_id()), "dotui node has UiLayout");
}
