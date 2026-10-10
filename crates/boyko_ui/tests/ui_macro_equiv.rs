//! P2 PRIMARY GATE — `ui!` ≡ hand-spawn STATE equivalence.
//!
//! For several representative trees we spawn the SAME tree three ways in one
//! world: through the `ui!` macro, through the insert-path baseline
//! (`Ui::spawn`), and through the bundle-path baseline (`Ui::spawn_via_bundle`).
//! After one apply window we assert the trees are state-identical:
//!
//! * same archetype id (POST-link: leaf / +ChildOf / +Children / +both) across
//!   all three paths;
//! * same component presence (incl. `UiName`, `ChildOf`, `Children`);
//! * same AUTHORED component values (drain-order-independent);
//! * same `ChildOf`/`Children` hierarchy (parent FK + child membership);
//! * every node live after apply.
//!
//! The `ui!` macro emits code against a `cmds: Commands` binding and returns the
//! root `Entity`; freshly-spawned handles are smuggled out of the `Send + Sync`
//! system closure through `Arc<Mutex<…>>` (the Phase-11/19 pattern). The macro's
//! `#named` `let` bindings are read INSIDE the closure (they do not escape it),
//! so the harvested `Entity` ids are what the test compares.

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Mutex};

use common::{NodeSpec, Ui};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};

use boyko_ui::components::{
    ComputedRect, ContentSize, UiAbsolute, UiAlign, UiLayout, UiName, UiRoot, UiSpacing,
};
use boyko_ui::prelude::ui;
use boyko_ui::units::{AlignMain, LayoutType, Unit};

fn col(w: Unit, h: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width: w, height: h, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

/// Assert two nodes have the same archetype id (post-link).
#[track_caller]
fn same_archetype(ui: &Ui, a: Entity, b: Entity, what: &str) {
    let aa = ui.archetype_of(a).expect("node a has an archetype");
    let bb = ui.archetype_of(b).expect("node b has an archetype");
    assert_eq!(aa, bb, "{what}: archetypes must match (dsl vs hand)");
}

/// Assert both nodes agree on presence of a component, and if present, on value.
///
/// Most layout components are POD `Copy` but do NOT derive `PartialEq`, so the
/// value comparison uses the derived `Debug` string (a total, deterministic
/// projection of every field for these plain structs) instead of `==`.
#[track_caller]
fn same_component<T: Component + std::fmt::Debug>(ui: &Ui, a: Entity, b: Entity, what: &str) {
    let av = ui.world.get_component::<T>(a);
    let bv = ui.world.get_component::<T>(b);
    assert_eq!(
        av.is_some(),
        bv.is_some(),
        "{what}: presence of {} must match",
        std::any::type_name::<T>()
    );
    if let (Some(x), Some(y)) = (av, bv) {
        assert_eq!(
            format!("{x:?}"),
            format!("{y:?}"),
            "{what}: value of {} must match",
            std::any::type_name::<T>()
        );
    }
}

/// Find the unique child of `parent` whose `UiName` is `name`. Children order is
/// unspecified, so we look the node up by its stable diff key.
#[track_caller]
fn child_named(ui: &Ui, parent: Entity, name: &str) -> Entity {
    let kids = ui.children_of(parent).unwrap_or_default();
    let mut found = None;
    for k in kids {
        if ui.name_of(k).map(|n| n.as_str() == name).unwrap_or(false) {
            assert!(found.is_none(), "two children named `{name}`");
            found = Some(k);
        }
    }
    found.unwrap_or_else(|| panic!("no child named `{name}` under the parent"))
}

// ───────────────────────────── 1. leaf ────────────────────────────────────

#[test]
fn dsl_leaf_equals_hand_leaf() {
    let mut ui = Ui::default_world();

    // DSL.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! { UiLayout { width: Unit::Px(120.0), height: Unit::Px(24.0), ..UiLayout::default() } };
        *probe.lock().unwrap() = Some(r);
    });
    let dsl = sink.lock().unwrap().expect("dsl root");

    // Hand (insert baseline). Same UiLayout; the macro injects ComputedRect.
    let hand = ui.spawn(NodeSpec::new(col(px(120.0), px(24.0))), None);

    same_archetype(&ui, dsl, hand, "leaf");
    same_component::<UiLayout>(&ui, dsl, hand, "leaf");
    same_component::<ComputedRect>(&ui, dsl, hand, "leaf"); // both present
    assert!(ui.world.has_entity(dsl), "dsl leaf live");
    // No optional components on either.
    assert!(ui.world.get_component::<UiName>(dsl).is_none(), "leaf has no UiName");
    assert!(ui.world.get_component::<ChildOf>(dsl).is_none(), "leaf has no ChildOf");
    assert!(ui.world.get_component::<Children>(dsl).is_none(), "leaf has no Children");
}

// ───────────────── 2. multi-component node → UiNodeBundle ──────────────────

#[test]
fn dsl_bundle_node_equals_hand_three_ways() {
    let mut ui = Ui::default_world();

    // DSL: set contains {UiLayout, ComputedRect} -> canonical UiNodeBundle.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiLayout { width: Unit::Px(300.0), ..UiLayout::default() },
            ComputedRect::default(),
            UiSpacing { padding_left: Unit::Px(8.0), ..UiSpacing::default() },
            UiAlign { main: AlignMain::Center, ..UiAlign::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let dsl = sink.lock().unwrap().expect("dsl root");

    // Insert-path baseline.
    let hand = ui.spawn(
        NodeSpec::new(UiLayout { width: px(300.0), ..UiLayout::default() })
            .with_spacing(UiSpacing { padding_left: px(8.0), ..UiSpacing::default() })
            .with_align(UiAlign { main: AlignMain::Center, ..UiAlign::default() }),
        None,
    );

    // Bundle-path baseline.
    let bundle = ui.spawn_via_bundle(
        NodeSpec::new(UiLayout { width: px(300.0), ..UiLayout::default() })
            .with_spacing(UiSpacing { padding_left: px(8.0), ..UiSpacing::default() })
            .with_align(UiAlign { main: AlignMain::Center, ..UiAlign::default() }),
        None,
    );

    // Three-way archetype identity.
    same_archetype(&ui, dsl, hand, "bundle-node dsl-vs-insert");
    same_archetype(&ui, dsl, bundle, "bundle-node dsl-vs-bundle");
    same_archetype(&ui, hand, bundle, "bundle-node insert-vs-bundle");

    same_component::<UiLayout>(&ui, dsl, hand, "bundle-node");
    same_component::<ComputedRect>(&ui, dsl, hand, "bundle-node");
    same_component::<UiSpacing>(&ui, dsl, hand, "bundle-node");
    same_component::<UiAlign>(&ui, dsl, hand, "bundle-node");
}

// ───────── 3. shuffled component order proves SET-based recognition ─────────

#[test]
fn dsl_bundle_recognition_is_set_based_not_positional() {
    let mut ui = Ui::default_world();

    // ComputedRect FIRST, UiLayout in the middle — still must take the bundle.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiSpacing { padding_top: Unit::Px(4.0), ..UiSpacing::default() },
            ComputedRect::default(),
            UiLayout { width: Unit::Px(300.0), ..UiLayout::default() }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let shuffled = sink.lock().unwrap().expect("shuffled root");

    // Canonical order, same set.
    let sink2: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe2 = Arc::clone(&sink2);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiLayout { width: Unit::Px(300.0), ..UiLayout::default() },
            ComputedRect::default(),
            UiSpacing { padding_top: Unit::Px(4.0), ..UiSpacing::default() }
        };
        *probe2.lock().unwrap() = Some(r);
    });
    let canonical = sink2.lock().unwrap().expect("canonical root");

    // Identical set, identical archetype regardless of author order.
    same_archetype(&ui, shuffled, canonical, "set-based recognition");
    same_component::<UiLayout>(&ui, shuffled, canonical, "set-based");
    same_component::<UiSpacing>(&ui, shuffled, canonical, "set-based");
    same_component::<ComputedRect>(&ui, shuffled, canonical, "set-based");
}

// ─────────────────────────── 4. two-level nest ────────────────────────────

#[test]
fn dsl_two_level_nest_equals_hand() {
    let mut ui = Ui::default_world();

    // The macro's `#named` bindings are scoped INSIDE the `ui!{}` block (the
    // expansion wraps everything in a block that evaluates to the root), so only
    // the root escapes. Children are harvested post-apply via the hierarchy and
    // matched by their `UiName` diff key.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let root = ui! {
            #r2_root {
                UiLayout { layout_type: LayoutType::Column, ..UiLayout::default() },
                UiRoot,
                children: [
                    #r2_a { UiLayout { height: Unit::Px(48.0), ..UiLayout::default() } },
                    #r2_b { UiLayout { height: Unit::Px(48.0), ..UiLayout::default() } }
                ]
            }
        };
        *probe.lock().unwrap() = Some(root);
    });
    let droot = sink.lock().unwrap().expect("dsl root");
    let da = child_named(&ui, droot, "r2_a");
    let db = child_named(&ui, droot, "r2_b");

    // Hand.
    let hroot = ui.spawn(
        NodeSpec::root(UiLayout { layout_type: LayoutType::Column, ..UiLayout::default() })
            .with_name("r2_root"),
        None,
    );
    let ha = ui.spawn(NodeSpec::new(col(Unit::Auto, px(48.0))).with_name("r2_a"), Some(hroot));
    let hb = ui.spawn(NodeSpec::new(col(Unit::Auto, px(48.0))).with_name("r2_b"), Some(hroot));

    // Archetype identity per role.
    same_archetype(&ui, droot, hroot, "nest root (+Children)");
    same_archetype(&ui, da, ha, "nest childA (+ChildOf)");
    same_archetype(&ui, db, hb, "nest childB (+ChildOf)");

    // Component values.
    same_component::<UiLayout>(&ui, da, ha, "childA");
    same_component::<UiName>(&ui, droot, hroot, "root name");

    // Hierarchy: every child's ChildOf points at the root.
    assert_eq!(ui.parent_of(da), Some(droot), "childA.ChildOf == root");
    assert_eq!(ui.parent_of(db), Some(droot), "childB.ChildOf == root");
    let kids = ui.children_of(droot).expect("root has Children");
    assert!(kids.contains(&da) && kids.contains(&db), "root.Children holds both kids");
    assert_eq!(kids.len(), 2, "root has exactly two children");
}

// ────────────────────── 5. three-level deep nest ──────────────────────────

#[test]
fn dsl_three_level_nest_equals_hand() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let gp = ui! {
            #r3_gp {
                UiLayout::default(),
                UiRoot,
                children: [
                    #r3_p {
                        UiLayout { height: Unit::Px(40.0), ..UiLayout::default() },
                        children: [
                            #r3_c { UiLayout { height: Unit::Px(10.0), ..UiLayout::default() } }
                        ]
                    }
                ]
            }
        };
        *probe.lock().unwrap() = Some(gp);
    });
    let dgp = sink.lock().unwrap().expect("dsl grandparent");
    let dp = child_named(&ui, dgp, "r3_p");
    let dc = child_named(&ui, dp, "r3_c");

    let hgp = ui.spawn(NodeSpec::root(UiLayout::default()).with_name("r3_gp"), None);
    let hp = ui.spawn(NodeSpec::new(col(Unit::Auto, px(40.0))).with_name("r3_p"), Some(hgp));
    let hc = ui.spawn(NodeSpec::new(col(Unit::Auto, px(10.0))).with_name("r3_c"), Some(hp));

    // Mid-tree node `p` is BOTH a parent and a child (+ChildOf +Children).
    same_archetype(&ui, dgp, hgp, "gp (+Children)");
    same_archetype(&ui, dp, hp, "p (+ChildOf +Children)");
    same_archetype(&ui, dc, hc, "c (+ChildOf)");

    // Hierarchy chain materialised in ONE apply window (link-integrity).
    assert_eq!(ui.parent_of(dp), Some(dgp), "p.ChildOf == gp");
    assert_eq!(ui.parent_of(dc), Some(dp), "c.ChildOf == p");
    assert!(ui.children_of(dgp).unwrap().contains(&dp), "gp.Children ∋ p");
    assert!(ui.children_of(dp).unwrap().contains(&dc), "p.Children ∋ c");
    assert!(ui.world.get_component::<Children>(dc).is_none(), "leaf c has no Children");
}

// ──────────────────────── 6. #named carries UiName ────────────────────────

#[test]
fn dsl_named_node_carries_uiname() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            #title_bar { UiLayout { height: Unit::Px(32.0), ..UiLayout::default() } }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let dsl = sink.lock().unwrap().expect("named root");

    let name = ui.name_of(dsl).expect("named node has UiName");
    assert_eq!(name.as_str(), "title_bar", "#name lowers to the right UiName string");

    // Hand node with the SAME name -> identical archetype + value.
    let hand = ui.spawn(NodeSpec::new(col(Unit::Auto, px(32.0))).with_name("title_bar"), None);
    same_archetype(&ui, dsl, hand, "named node");
    same_component::<UiName>(&ui, dsl, hand, "named node UiName value");
}

// ────────────── 7. node with every optional component ──────────────────────

#[test]
fn dsl_full_optional_set_equals_hand() {
    let mut ui = Ui::default_world();

    let spacing = UiSpacing { padding_left: px(3.0), row_gap: px(2.0), ..UiSpacing::default() };
    let align = UiAlign { main: AlignMain::Center, ..UiAlign::default() };
    let absolute = UiAbsolute { left: px(5.0), ..UiAbsolute::default() };
    let content = ContentSize { width: 12.0, height: 7.0 };

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
                UiRoot
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let dsl = sink.lock().unwrap().expect("full-set root");

    let hand = ui.spawn(
        NodeSpec::root(UiLayout { width: px(50.0), ..UiLayout::default() })
            .with_spacing(spacing)
            .with_align(align)
            .with_absolute(absolute)
            .with_content(content)
            .with_name("everything"),
        None,
    );

    same_archetype(&ui, dsl, hand, "full optional set");
    same_component::<UiLayout>(&ui, dsl, hand, "full-set UiLayout");
    same_component::<ComputedRect>(&ui, dsl, hand, "full-set ComputedRect");
    same_component::<UiSpacing>(&ui, dsl, hand, "full-set UiSpacing");
    same_component::<UiAlign>(&ui, dsl, hand, "full-set UiAlign");
    same_component::<UiAbsolute>(&ui, dsl, hand, "full-set UiAbsolute");
    same_component::<ContentSize>(&ui, dsl, hand, "full-set ContentSize");
    same_component::<UiName>(&ui, dsl, hand, "full-set UiName");
    // UiRoot is a ZST tag — presence only.
    assert_eq!(
        ui.world.has_component(dsl, UiRoot::component_id()),
        ui.world.has_component(hand, UiRoot::component_id()),
        "full-set UiRoot presence matches"
    );
}

// ──────────── 8. multiple top-level roots → tuple of Entity ────────────────

#[test]
fn dsl_multiple_roots_return_tuple() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<(Entity, Entity)>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let (a, b) = ui! {
            #root_one { UiLayout { width: Unit::Px(10.0), ..UiLayout::default() } },
            #root_two { UiLayout { width: Unit::Px(20.0), ..UiLayout::default() } }
        };
        *probe.lock().unwrap() = Some((a, b));
    });
    let (a, b) = sink.lock().unwrap().expect("two roots");

    assert!(ui.world.has_entity(a) && ui.world.has_entity(b), "both roots live");
    assert_ne!(a, b, "the two roots are distinct entities");
    assert_eq!(ui.name_of(a).unwrap().as_str(), "root_one", "first root named");
    assert_eq!(ui.name_of(b).unwrap().as_str(), "root_two", "second root named");
    // Neither is the other's child.
    assert!(ui.world.get_component::<ChildOf>(a).is_none(), "root_one has no parent");
    assert!(ui.world.get_component::<ChildOf>(b).is_none(), "root_two has no parent");
}
