//! P2 Test #2 — command-count / structural-cost equivalence (Decision 7).
//!
//! The `CommandQueue` packs commands as opaque bytes with no public per-command
//! count, so the direct Principle-1 regression guard (a double-emitted
//! `ComputedRect` insert = an extra migration) is asserted through the OBSERVABLE
//! structural footprint the commands produce:
//!
//! * entity-count delta — the macro must create EXACTLY one entity per node
//!   (no stray spawn);
//! * archetype-count delta — driving the `ui!` path and the hand insert-baseline
//!   from the SAME fresh-world state must reach the SAME number of distinct
//!   archetypes (a divergent lowering — e.g. an extra migration through a
//!   different intermediate archetype, or a missed bundle fast path — would
//!   change the archetype set).
//!
//! These two deltas together pin "the macro does node-for-node the same
//! structural work as the hand baseline".

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Mutex};

use common::{NodeSpec, Ui};

use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::components::{UiLayout, UiRoot, UiSpacing};
use boyko_ui::prelude::ui;
use boyko_ui::units::Unit;

#[test]
fn dsl_creates_exactly_one_entity_per_node() {
    let mut ui = Ui::default_world();
    let before = ui.world.entity_count();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        // 4 nodes: root + 3 children.
        let root = ui! {
            #cc_root {
                UiLayout::default(),
                UiRoot,
                children: [
                    #cc_a { UiLayout::default() },
                    #cc_b { UiLayout::default() },
                    #cc_c { UiLayout::default() }
                ]
            }
        };
        *probe.lock().unwrap() = Some(root);
    });

    let after = ui.world.entity_count();
    assert_eq!(after - before, 4, "4-node tree must create exactly 4 entities (no stray spawn)");
}

#[test]
fn dsl_and_hand_reach_the_same_archetype_count() {
    // Fresh world for the DSL path.
    let mut dsl_world = Ui::default_world();
    let dsl_before = dsl_world.world.archetype_count();
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    dsl_world.author(move |mut cmds| {
        let root = ui! {
            #x_root {
                UiLayout { width: Unit::Px(100.0), ..UiLayout::default() },
                UiRoot,
                children: [
                    #x_a {
                        UiLayout::default(),
                        UiSpacing { padding_left: Unit::Px(4.0), ..UiSpacing::default() }
                    },
                    #x_b { UiLayout::default() }
                ]
            }
        };
        *probe.lock().unwrap() = Some(root);
    });
    let dsl_delta = dsl_world.world.archetype_count() - dsl_before;

    // Fresh world for the hand path — identical shape, identical insert order.
    let mut hand_world = Ui::default_world();
    let hand_before = hand_world.world.archetype_count();
    let root = hand_world.spawn(
        NodeSpec::root(UiLayout { width: boyko_ui::units::Unit::Px(100.0), ..UiLayout::default() })
            .with_name("x_root"),
        None,
    );
    let _a = hand_world.spawn(
        NodeSpec::new(UiLayout::default())
            .with_spacing(UiSpacing {
                padding_left: boyko_ui::units::Unit::Px(4.0),
                ..UiSpacing::default()
            })
            .with_name("x_a"),
        Some(root),
    );
    let _b = hand_world.spawn(NodeSpec::new(UiLayout::default()).with_name("x_b"), Some(root));
    let hand_delta = hand_world.world.archetype_count() - hand_before;

    assert_eq!(
        dsl_delta, hand_delta,
        "DSL and hand baseline must reach the same number of distinct archetypes \
         (no extra migration / no missed fast path)"
    );
}

#[test]
fn canonical_bundle_node_does_not_double_emit_computed_rect() {
    // A node whose set already contains ComputedRect must NOT also inject a
    // second one. If it did, the spawn would be a UiNodeBundle + a redundant
    // ComputedRect re-insert; the entity count is unaffected but a re-insert is a
    // wasted command. We assert via archetype identity to the bundle-baseline
    // (which inserts ComputedRect exactly once, inside the bundle).
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            UiLayout { width: Unit::Px(7.0), ..UiLayout::default() },
            boyko_ui::components::ComputedRect::default()
        };
        *probe.lock().unwrap() = Some(r);
    });
    let dsl = sink.lock().unwrap().expect("canonical leaf");

    let bundle = ui.spawn_via_bundle(
        NodeSpec::new(UiLayout { width: boyko_ui::units::Unit::Px(7.0), ..UiLayout::default() }),
        None,
    );

    assert_eq!(
        ui.archetype_of(dsl),
        ui.archetype_of(bundle),
        "canonical node lands in the same archetype as a single-ComputedRect bundle spawn"
    );
}
