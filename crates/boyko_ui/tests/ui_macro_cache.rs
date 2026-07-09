//! P2 Test #3 — bundle-cache-PATH verification (Decision 7).
//!
//! Asserts the `ui!` canonical node takes the `UiNodeBundle` fast path (hits the
//! Phase-8.5 static archetype cache slot), not a `spawn(UiLayout) +
//! insert(ComputedRect)` migration that merely converges to the same archetype.
//!
//! The probe: a canonical LEAF (set = {UiLayout, ComputedRect}, no children, no
//! parent) takes `UiNodeBundle` as its FINAL archetype. After spawning it via
//! `ui!`, `UiNodeBundle::cached_archetype_id(&mut world)` returns the cached
//! slot's id — and it must equal the leaf's archetype id. Equality proves the
//! spawn resolved through the `UiNodeBundle` `BundleTypeId` cache (the fast
//! path), since `cached_archetype_id` reads the same per-world `OnceLock` slot
//! the spawn populated.

mod common;

use std::sync::{Arc, Mutex};

use common::Ui;

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::bundles::UiNodeBundle;
use boyko_ui::prelude::ui;
use boyko_ui::units::Unit;

fn spawn_canonical_leaf(ui: &mut Ui) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        let r = ui! {
            boyko_ui::components::UiLayout { width: Unit::Px(0.0), ..boyko_ui::components::UiLayout::default() },
            boyko_ui::components::ComputedRect::default()
        };
        *probe.lock().unwrap() = Some(r);
    });
    sink.lock().unwrap().expect("canonical leaf")
}

#[test]
fn ui_canonical_node_hits_uinodebundle_cache_slot() {
    let mut ui = Ui::default_world();

    let leaf = spawn_canonical_leaf(&mut ui);
    let leaf_arch = ui.archetype_of(leaf).expect("leaf has an archetype");

    // The cached `UiNodeBundle` archetype slot. This reads the SAME per-world
    // OnceLock the `ui!` spawn populated; if the spawn had taken the slow
    // (spawn UiLayout + insert ComputedRect) path, this call would COLD-resolve
    // the bundle archetype now — and for a single-bundle world that cold archetype
    // would be a DISTINCT id from the leaf's migration archetype.
    let cached = UiNodeBundle::cached_archetype_id(&mut ui.world);

    assert_eq!(
        leaf_arch, cached,
        "canonical ui! leaf must land in the UiNodeBundle cache-slot archetype \
         (the bundle fast path executed, not a UiLayout+ComputedRect migration)"
    );
}

#[test]
fn all_same_shape_canonical_nodes_share_one_archetype() {
    let mut ui = Ui::default_world();

    let mut leaves = Vec::new();
    for _ in 0..16 {
        leaves.push(spawn_canonical_leaf(&mut ui));
    }

    let first = ui.archetype_of(leaves[0]).expect("first leaf archetype");
    for (i, &e) in leaves.iter().enumerate() {
        let a = ui.archetype_of(e).expect("leaf archetype");
        assert_eq!(a, first, "leaf {i} shares the one canonical archetype (warm cache)");
    }

    let cached = UiNodeBundle::cached_archetype_id(&mut ui.world);
    assert_eq!(first, cached, "the shared archetype is the UiNodeBundle cache slot");
}

#[test]
fn warm_cache_does_not_grow_archetype_count() {
    let mut ui = Ui::default_world();

    // First spawn warms the cache (cold archetype creation).
    let _ = spawn_canonical_leaf(&mut ui);
    let after_first = ui.world.archetype_count();

    // Subsequent identical-shape spawns must NOT create new archetypes.
    for _ in 0..8 {
        let _ = spawn_canonical_leaf(&mut ui);
    }
    let after_more = ui.world.archetype_count();

    assert_eq!(
        after_first, after_more,
        "identical-shape canonical spawns reuse the cached archetype (no growth)"
    );
}
