//! P2 Test #4 — single-window deep-tree link integrity (Decision 6 / critic-2 #2).
//!
//! The load-bearing ordering invariant: within ONE apply window, every parent's
//! spawn is enqueued before its descendants' `ChildOf` insert (pre-order DFS
//! lowering). The `child_of_on_insert` hook silently DROPS a `ChildOf` whose
//! parent is not live at apply time and never retries; if the macro emitted a
//! child's link before the parent's spawn, the link would vanish.
//!
//! These tests build the whole tree in a SINGLE `ui!` invocation (one apply
//! window) and assert every `ChildOf`/`Children` edge materialised.

mod common;

use std::sync::{Arc, Mutex};

use common::Ui;

use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;

use boyko_ui::components::{UiLayout, UiRoot};
use boyko_ui::prelude::ui;

/// Harvest a named child of `parent`.
fn named_child(ui: &Ui, parent: Entity, name: &str) -> Entity {
    ui.children_of(parent)
        .unwrap_or_default()
        .into_iter()
        .find(|&k| ui.name_of(k).map(|n| n.as_str() == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("no child named `{name}`"))
}

#[test]
fn three_level_chain_links_in_one_window() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let gp = ui! {
            #w_gp {
                UiLayout::default(),
                UiRoot,
                children: [
                    #w_p {
                        UiLayout::default(),
                        children: [
                            #w_c {
                                UiLayout::default(),
                                children: [
                                    #w_gc { UiLayout::default() }
                                ]
                            }
                        ]
                    }
                ]
            }
        };
        *probe.lock().unwrap() = Some(gp);
    });
    let gp = sink.lock().unwrap().expect("grandparent");
    let p = named_child(&ui, gp, "w_p");
    let c = named_child(&ui, p, "w_c");
    let gc = named_child(&ui, c, "w_gc");

    // Every FK materialised — no link silently dropped by the dangling guard.
    assert_eq!(ui.parent_of(p), Some(gp), "p -> gp");
    assert_eq!(ui.parent_of(c), Some(p), "c -> p");
    assert_eq!(ui.parent_of(gc), Some(c), "gc -> c");

    // Every reverse collection holds the right child.
    assert!(ui.children_of(gp).unwrap().contains(&p), "gp.Children ∋ p");
    assert!(ui.children_of(p).unwrap().contains(&c), "p.Children ∋ c");
    assert!(ui.children_of(c).unwrap().contains(&gc), "c.Children ∋ gc");
    assert!(ui.world.get_component::<Children>(gc).is_none(), "leaf gc has no Children");
}

#[test]
fn wide_fanout_links_all_in_one_window() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let root = ui! {
            #fan_root {
                UiLayout::default(),
                UiRoot,
                children: [
                    #fan_0 { UiLayout::default() },
                    #fan_1 { UiLayout::default() },
                    #fan_2 { UiLayout::default() },
                    #fan_3 { UiLayout::default() },
                    #fan_4 { UiLayout::default() }
                ]
            }
        };
        *probe.lock().unwrap() = Some(root);
    });
    let root = sink.lock().unwrap().expect("fan root");

    let kids = ui.children_of(root).expect("root has Children");
    assert_eq!(kids.len(), 5, "all five children linked in one window");
    for i in 0..5 {
        let name = format!("fan_{i}");
        let child = named_child(&ui, root, &name);
        assert_eq!(ui.parent_of(child), Some(root), "{name}.ChildOf == root");
    }
}

#[test]
fn deep_six_level_chain_links_in_one_window() {
    let mut ui = Ui::default_world();

    // A 6-deep single chain — recursion-depth + ordering smoke in one window.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let l0 = ui! {
            #d0 {
                UiLayout::default(), UiRoot,
                children: [ #d1 {
                    UiLayout::default(),
                    children: [ #d2 {
                        UiLayout::default(),
                        children: [ #d3 {
                            UiLayout::default(),
                            children: [ #d4 {
                                UiLayout::default(),
                                children: [ #d5 { UiLayout::default() } ]
                            } ]
                        } ]
                    } ]
                } ]
            }
        };
        *probe.lock().unwrap() = Some(l0);
    });
    let mut cur = sink.lock().unwrap().expect("d0");
    for depth in 1..=5 {
        let name = format!("d{depth}");
        let next = named_child(&ui, cur, &name);
        assert_eq!(ui.parent_of(next), Some(cur), "d{depth}.ChildOf == d{}", depth - 1);
        cur = next;
    }
    // The deepest leaf has no Children.
    assert!(ui.world.get_component::<Children>(cur).is_none(), "d5 leaf has no Children");
}
