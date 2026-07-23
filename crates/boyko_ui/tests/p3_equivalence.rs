//! GATE 1 (PRIMARY) — `.ui` parse+lower ≡ `ui!` macro on INITIAL load.
//!
//! For several representative trees we build the entity tree TWICE in one world:
//! once by parsing + lowering a canonical `.ui` source, once by the P2 `ui!`
//! macro. We then assert the two subtrees are structurally identical:
//!
//! * same entity count;
//! * same component-id SET per node (`UiSourceOrder` excluded by construction —
//!   it is crate-private and the macro never stamps it, Decision 12);
//! * same component BYTE values per node (via the `Debug` projection, the gate's
//!   comparison method for the POD layout components);
//! * same `ChildOf`/`Children` topology AND initial child order (pairwise by slot
//!   — both paths order `add_child` by declaration order through the FIFO drain);
//! * same `UiName`.
//!
//! The `ui!` side is driven exactly as `ui_macro_equiv.rs` does: a `Commands`
//! closure, the root handle smuggled out via `Arc<Mutex<…>>`; the `.ui` side
//! through the `spawn_dot_ui` harness helper.

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
use p3_common::{assert_subtree_equiv, spawn_dot_ui, subtree_count};

use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::components::{
    ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign, UiLayout, UiRoot, UiSpacing,
};
use boyko_ui::prelude::ui;
use boyko_ui::units::{AlignMain, LayoutType, Unit};

// ───────────────────────────── 1. leaf ────────────────────────────────────

#[test]
fn dot_ui_leaf_equals_macro() {
    let mut ui = Ui::default_world();

    // ui! side.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! { UiLayout { width: Unit::Px(120.0), height: Unit::Px(24.0), ..UiLayout::default() } };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    // .ui side. A bare UiLayout-only node — the UiLayout-only spawn base (the
    // macro injects ComputedRect::default() too), so the component SET matches.
    let src = "\
version=1
#leaf  UiLayout { layout_type: Column, width: Px(120), height: Px(24) }
";
    // The macro node is unnamed; give the .ui node a name only if we compare a
    // named macro node. Here the macro node is unnamed, so make the .ui node
    // unnamed too (an anonymous head with a bare UiLayout component).
    let src_anon = "\
version=1
UiLayout { layout_type: Column, width: Px(120), height: Px(24) }
";
    let _ = src;
    let roots = spawn_dot_ui(&mut ui.world, src_anon);
    assert_eq!(roots.len(), 1, "one .ui root");
    let dot_root = roots[0];

    assert_eq!(
        subtree_count(&ui.world, dot_root),
        subtree_count(&ui.world, macro_root),
        "leaf entity count must match"
    );
    assert_subtree_equiv(&ui.world, dot_root, macro_root, "leaf");
}

// ───────────────── 2. multi-component node (bundle fast path) ──────────────

#[test]
fn dot_ui_bundle_node_equals_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #panel {
                UiLayout { width: Unit::Px(300.0), ..UiLayout::default() },
                ComputedRect::default(),
                UiSpacing { padding_left: Unit::Px(8.0), ..UiSpacing::default() },
                UiAlign { main: AlignMain::Center, ..UiAlign::default() }
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    // .ui: same component SET incl. an explicit ComputedRect (bundle fast path).
    // The macro's `UiLayout { width: Px(300), ..default }` keeps Column default
    // layout_type, Auto height etc., so spell the .ui body to the same defaults.
    let src = "\
version=1
#panel  UiLayout { layout_type: Column, width: Px(300), height: Auto }
    ComputedRect { x: 0, y: 0, w: 0, h: 0 }
    UiSpacing { padding_left: Px(8) }
    UiAlign { main: Center }
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    let dot_root = roots[0];

    assert_subtree_equiv(&ui.world, dot_root, macro_root, "bundle-node");
}

// ─────────────────────────── 3. two-level nest ────────────────────────────

#[test]
fn dot_ui_two_level_nest_equals_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #root {
                UiLayout { layout_type: LayoutType::Column, ..UiLayout::default() },
                UiRoot,
                children: [
                    #a { UiLayout { height: Unit::Px(48.0), ..UiLayout::default() } },
                    #b { UiLayout { height: Unit::Px(48.0), ..UiLayout::default() } }
                ]
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    let src = "\
version=1
#root  UiLayout { layout_type: Column }
    UiRoot
    #a  UiLayout { layout_type: Column, height: Px(48) }
    #b  UiLayout { layout_type: Column, height: Px(48) }
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    let dot_root = roots[0];

    assert_eq!(
        subtree_count(&ui.world, dot_root),
        subtree_count(&ui.world, macro_root),
        "two-level entity count must match (3 each)"
    );
    assert_subtree_equiv(&ui.world, dot_root, macro_root, "two-level-nest");
}

// ────────────────────── 4. three-level deep nest ──────────────────────────

#[test]
fn dot_ui_three_level_nest_equals_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #gp {
                UiLayout::default(),
                UiRoot,
                children: [
                    #p {
                        UiLayout { height: Unit::Px(40.0), ..UiLayout::default() },
                        children: [
                            #c { UiLayout { height: Unit::Px(10.0), ..UiLayout::default() } }
                        ]
                    }
                ]
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    let src = "\
version=1
#gp  UiLayout { layout_type: Column }
    UiRoot
    #p  UiLayout { layout_type: Column, height: Px(40) }
        #c  UiLayout { layout_type: Column, height: Px(10) }
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    let dot_root = roots[0];

    assert_eq!(subtree_count(&ui.world, dot_root), 3, ".ui three-level count is 3");
    assert_subtree_equiv(&ui.world, dot_root, macro_root, "three-level-nest");
}

// ─────────────────── 5. node with every optional component ─────────────────

#[test]
fn dot_ui_full_optional_set_equals_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #everything {
                UiLayout { width: Unit::Px(50.0), ..UiLayout::default() },
                ComputedRect::default(),
                UiSpacing { padding_left: Unit::Px(3.0), row_gap: Unit::Px(2.0), ..UiSpacing::default() },
                UiAlign { main: AlignMain::Center, ..UiAlign::default() },
                UiAbsolute { left: Unit::Px(5.0), ..UiAbsolute::default() },
                ContentSize { width: 12.0, height: 7.0 },
                StackIndex(10),
                UiRoot
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    let src = "\
version=1
#everything  UiLayout { layout_type: Column, width: Px(50) }
    ComputedRect { x: 0, y: 0, w: 0, h: 0 }
    UiSpacing { padding_left: Px(3), row_gap: Px(2) }
    UiAlign { main: Center }
    UiAbsolute { left: Px(5) }
    ContentSize { width: 12, height: 7 }
    StackIndex(10)
    UiRoot
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    let dot_root = roots[0];

    assert_subtree_equiv(&ui.world, dot_root, macro_root, "full-optional-set");
}

// ──────────────── 6. fractional float values round to identical bits ───────

#[test]
fn dot_ui_fractional_floats_equal_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #frac {
                UiLayout { width: Unit::Pct(33.5), height: Unit::Stretch(1.5), ..UiLayout::default() },
                ContentSize { width: 12.25, height: 7.75 }
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let macro_root = sink.lock().unwrap().expect("macro root");

    let src = "\
version=1
#frac  UiLayout { layout_type: Column, width: Pct(33.5), height: Stretch(1.5) }
    ContentSize { width: 12.25, height: 7.75 }
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    let dot_root = roots[0];

    assert_subtree_equiv(&ui.world, dot_root, macro_root, "fractional-floats");
}

// ──────────────── 7. multiple top-level roots, declaration order ───────────

#[test]
fn dot_ui_multiple_roots_equal_macro() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<(Entity, Entity)>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let (a, b) = ui! {
            #r1 { UiLayout { width: Unit::Px(10.0), ..UiLayout::default() } },
            #r2 { UiLayout { width: Unit::Px(20.0), ..UiLayout::default() } }
        };
        *probe.lock().unwrap() = Some((a, b));
    });
    let (ma, mb) = sink.lock().unwrap().expect("two macro roots");

    let src = "\
version=1
#r1  UiLayout { layout_type: Column, width: Px(10) }
#r2  UiLayout { layout_type: Column, width: Px(20) }
";
    let roots = spawn_dot_ui(&mut ui.world, src);
    assert_eq!(roots.len(), 2, "two .ui roots in declaration order");
    assert_subtree_equiv(&ui.world, roots[0], ma, "root1");
    assert_subtree_equiv(&ui.world, roots[1], mb, "root2");
}
