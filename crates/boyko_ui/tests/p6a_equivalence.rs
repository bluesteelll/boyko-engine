//! GATE 6 — `ui!`-authored == `.ui`-authored == hand-spawned widget tree.
//!
//! The P2/P3 equivalence bar (entity tree state-identical across the three
//! authoring paths), EXTENDED to the GUI P6a widget vocabulary. The shared
//! `p3_common::assert_subtree_equiv` only covers the P1 layout components, so this
//! file carries its own presence + value comparator that ALSO covers the widget
//! components (`Button`/`Bar`/`BarFill` markers, `UiImage`, `UiGrid`, `UiAnchor`,
//! `OnClick`).
//!
//! Each tree is authored as the explicit CANONICAL component set (the form the
//! `ui!` macro and the `.ui` closed-match both lower), in the intersection of what
//! all three paths can author: the `.ui` closed match (`dispatch.rs`) covers the
//! widget markers + `UiImage`/`UiGrid`/`UiAnchor` + `OnClick(n)`, so the trees use
//! exactly those. (`Interaction`/`Focusable` are not `.ui`-authorable — a
//! documented seam — so they are out of the cross-form equivalence set; the
//! Button-dispatch gate exercises them directly.)

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;
mod p3_common;

use std::sync::{Arc, Mutex};

use common::Ui;
use p3_common::spawn_dot_ui;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::components::{
    AnchorEdge, Bar, BarFill, Button, ComputedRect, UiAnchor, UiGrid, UiImage, UiLayout, UiRoot,
};
use boyko_ui::interaction::action::OnClick;
use boyko_ui::prelude::ui;
use boyko_ui::units::{LayoutType, Unit};

/// Presence + value comparator covering the P6a widget components (and the layout
/// base). Markers are presence-only; struct/tuple components are compared by their
/// `Debug` byte projection (the P3 method).
#[track_caller]
fn assert_widget_node_equiv(world: &EcsMaster, a: Entity, b: Entity, what: &str) {
    // Marker / base presence.
    macro_rules! pres {
        ($t:ty, $name:literal) => {
            assert_eq!(
                world.has_component(a, <$t>::component_id()),
                world.has_component(b, <$t>::component_id()),
                "{what}: presence of {} must match",
                $name
            );
        };
    }
    pres!(UiLayout, "UiLayout");
    pres!(ComputedRect, "ComputedRect");
    pres!(Button, "Button");
    pres!(Bar, "Bar");
    pres!(BarFill, "BarFill");
    pres!(UiImage, "UiImage");
    pres!(UiGrid, "UiGrid");
    pres!(UiAnchor, "UiAnchor");
    pres!(OnClick, "OnClick");
    pres!(UiRoot, "UiRoot");

    // Value comparison (Debug projection) for the struct/tuple components present.
    macro_rules! valeq {
        ($t:ty, $name:literal) => {
            let av = world.get_component::<$t>(a).map(|v| format!("{v:?}"));
            let bv = world.get_component::<$t>(b).map(|v| format!("{v:?}"));
            assert_eq!(av, bv, "{what}: value of {} must match", $name);
        };
    }
    valeq!(UiLayout, "UiLayout");
    valeq!(UiImage, "UiImage");
    valeq!(UiGrid, "UiGrid");
    valeq!(UiAnchor, "UiAnchor");
    valeq!(OnClick, "OnClick");
}

/// Children of a node in slice order, or empty.
fn children(world: &EcsMaster, e: Entity) -> Vec<Entity> {
    world
        .get_component::<boyko_ecs::ecs::core::hierarchy::Children>(e)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

// ───────────────────────── 1. Button widget ───────────────────────────────

#[test]
fn button_widget_three_ways_equivalent() {
    let mut ui = Ui::default_world();

    // ui!: explicit canonical Button set (marker + background + layout + onclick).
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            Button,
            boyko_ui::components::UiBackground::default(),
            OnClick(5u16),
            UiLayout { layout_type: LayoutType::Column, width: Unit::Px(120.0), height: Unit::Px(40.0), ..UiLayout::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro button");

    // .ui: the SAME canonical set via the closed-match dispatch.
    let src = "\
version=1
#btn  UiLayout { layout_type: Column, width: Px(120), height: Px(40) }
    Button
    UiBackground { color: 0 }
    OnClick(5)
";
    let dot = spawn_dot_ui(&mut ui.world, src);
    assert_eq!(dot.len(), 1, "one .ui button root");
    let dot_root = dot[0];

    // hand-spawn: insert the same components directly through Commands.
    let hsink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let hprobe = Arc::clone(&hsink);
    ui.author(move |mut cmds| {
        let mut ec = cmds.spawn(UiLayout {
            layout_type: LayoutType::Column,
            width: Unit::Px(120.0),
            height: Unit::Px(40.0),
            ..UiLayout::default()
        });
        ec.insert(ComputedRect::default());
        ec.insert(Button);
        ec.insert(boyko_ui::components::UiBackground::default());
        ec.insert(OnClick(5u16));
        *hprobe.lock().unwrap() = Some(ec.id());
    });
    let hand = hsink.lock().unwrap().expect("hand button");

    assert_widget_node_equiv(&ui.world, macro_root, dot_root, "Button ui!-vs-.ui");
    assert_widget_node_equiv(&ui.world, macro_root, hand, "Button ui!-vs-hand");
    assert_widget_node_equiv(&ui.world, dot_root, hand, "Button .ui-vs-hand");
}

// ───────────────────────── 2. Image widget ────────────────────────────────

#[test]
fn image_widget_three_ways_equivalent() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiImage { texture: 7u32, uv_min: [0.0f32, 0.0f32], uv_max: [1.0f32, 1.0f32], tint: 0xFFFFFFFFu32 },
            UiLayout { layout_type: LayoutType::Column, width: Unit::Px(64.0), height: Unit::Px(64.0), ..UiLayout::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro image");

    let src = "\
version=1
#img  UiLayout { layout_type: Column, width: Px(64), height: Px(64) }
    UiImage { texture: 7, uv_min: [0, 0], uv_max: [1, 1], tint: 4294967295 }
";
    let dot = spawn_dot_ui(&mut ui.world, src);
    let dot_root = dot[0];

    assert_widget_node_equiv(&ui.world, macro_root, dot_root, "Image ui!-vs-.ui");
    // Spot-check the value carried through.
    assert_eq!(
        ui.world.get_component::<UiImage>(macro_root).unwrap().texture,
        7,
        "image texture handle authored"
    );
}

// ───────────────────────── 3. Grid widget ─────────────────────────────────

#[test]
fn grid_widget_three_ways_equivalent() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiGrid { columns: 3u8, rows: 2u8 },
            UiLayout { layout_type: LayoutType::Grid, width: Unit::Px(300.0), height: Unit::Px(200.0), ..UiLayout::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro grid");

    let src = "\
version=1
#grid  UiLayout { layout_type: Grid, width: Px(300), height: Px(200) }
    UiGrid { columns: 3, rows: 2 }
";
    let dot = spawn_dot_ui(&mut ui.world, src);
    let dot_root = dot[0];

    assert_widget_node_equiv(&ui.world, macro_root, dot_root, "Grid ui!-vs-.ui");
    assert_eq!(
        ui.world.get_component::<UiGrid>(macro_root).unwrap().columns,
        3,
        "grid columns authored"
    );
}

// ───────────────────────── 4. Anchored widget ─────────────────────────────

#[test]
fn anchor_widget_three_ways_equivalent() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiAnchor { edge: AnchorEdge::BottomRight, offset_x: 16.0f32, offset_y: 24.0f32, use_safe_area: true, _pad: [0u8; 3] },
            UiRoot,
            UiLayout { layout_type: LayoutType::Column, width: Unit::Px(200.0), height: Unit::Px(100.0), ..UiLayout::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro anchor");

    let src = "\
version=1
#hud  UiLayout { layout_type: Column, width: Px(200), height: Px(100) }
    UiAnchor { edge: BottomRight, offset_x: 16, offset_y: 24, use_safe_area: true }
    UiRoot
";
    let dot = spawn_dot_ui(&mut ui.world, src);
    let dot_root = dot[0];

    assert_widget_node_equiv(&ui.world, macro_root, dot_root, "Anchor ui!-vs-.ui");
    let a = ui.world.get_component::<UiAnchor>(macro_root).unwrap();
    assert_eq!(a.edge, AnchorEdge::BottomRight, "anchor edge authored");
    assert!(a.use_safe_area, "anchor safe-area flag authored");
}

// ───────────────────────── 5. Bar track + fill nest ───────────────────────

#[test]
fn bar_widget_nest_ui_vs_dot_ui_equivalent() {
    // A Bar TRACK (marker + value-less here, just the marker + layout) with a
    // BarFill child — the nest the bar driver expects. `UiValue`/`BindValue` are a
    // documented `.ui` deferral, so the equivalence covers the marker + layout
    // structure (the authorable substrate), with the value supplied at runtime.
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #track {
                Bar,
                UiLayout { layout_type: LayoutType::Row, width: Unit::Px(200.0), height: Unit::Px(20.0), ..UiLayout::default() },
                children: [
                    #fill {
                        BarFill,
                        UiLayout { layout_type: LayoutType::Row, width: Unit::Pct(0.0), height: Unit::Px(20.0), ..UiLayout::default() }
                    }
                ]
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro bar");

    let src = "\
version=1
#track  UiLayout { layout_type: Row, width: Px(200), height: Px(20) }
    Bar
    #fill  UiLayout { layout_type: Row, width: Pct(0), height: Px(20) }
        BarFill
";
    let dot = spawn_dot_ui(&mut ui.world, src);
    let dot_root = dot[0];

    // Roots equivalent.
    assert_widget_node_equiv(&ui.world, macro_root, dot_root, "Bar track ui!-vs-.ui");
    // One fill child each, equivalent.
    let mc = children(&ui.world, macro_root);
    let dc = children(&ui.world, dot_root);
    assert_eq!(mc.len(), 1, "ui! track has one fill child");
    assert_eq!(dc.len(), 1, ".ui track has one fill child");
    assert_widget_node_equiv(&ui.world, mc[0], dc[0], "BarFill ui!-vs-.ui");
    assert!(ui.world.has_component(mc[0], BarFill::component_id()), "ui! fill carries BarFill");
    assert!(ui.world.has_component(dc[0], BarFill::component_id()), ".ui fill carries BarFill");
}
