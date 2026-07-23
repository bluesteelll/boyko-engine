//! Miri UB backstop for the boyko_ui layout pair (P1 has NO `unsafe`, so this is
//! a cheap correctness/aliasing check, not the primary gate).
//!
//! The lib unit tests drive a parallel [`Schedule`], whose threadpool worker-join
//! is impractically slow / stall-prone under Miri (it exercises the engine's
//! lock-free pool, validated separately under loom + Miri in the Phase-9 series).
//! This file instead drives the SAME two layout systems WITHOUT the threadpool —
//! each via `EcsMaster::run_system` (which runs a one-shot system on `&mut self`,
//! no pool). A freshly-built system's `last_run` is `current - MAX_CHANGE_AGE`, so
//! `ui_layout_discovery` observes the freshly-spawned tree as changed on its first
//! run and sets `dirty`; `ui_layout_apply` (an exclusive `fn(&mut EcsMaster)`)
//! then performs the real work the Miri check cares about: the `mem::take` scratch
//! borrow protocol + the recursive nested `get_component` / `get_component_mut`
//! walk. Trees are kept small so Miri stays fast.
//!
//! Run: `RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu cargo miri test -p
//! boyko-ui --test miri_layout` (the repo's `.cargo/config.toml` already sets
//! `MIRIFLAGS=-Zmiri-tree-borrows`).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiViewport};
use boyko_ui::units::{LayoutType, Unit};

fn col(width: Unit, height: Unit) -> UiLayout {
    UiLayout { layout_type: LayoutType::Column, width, height, ..UiLayout::default() }
}
fn px(v: f32) -> Unit {
    Unit::Px(v)
}

/// Spawns a node (UiLayout + ComputedRect, optional UiRoot/parent) via the
/// deferred queue on `&mut world` (no pool) and returns its live handle.
fn spawn(world: &mut EcsMaster, layout: UiLayout, parent: Option<Entity>, root: bool) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(layout);
        e.insert(ComputedRect::default());
        if root {
            e.insert(UiRoot);
        }
        if let Some(p) = parent {
            e.set_parent(p);
        }
        *probe.lock().expect("probe") = Some(e.id());
    });
    sink.lock().expect("probe").expect("handle")
}

/// Drives one discovery+apply pass via `run_system` (no threadpool).
///
/// To force the relayout deterministically WITHOUT relying on the per-system
/// `Changed` tick window (which `run_system` — unlike `Schedule::run` — does not
/// advance the same way), we bump the viewport `generation` first. Discovery sets
/// `dirty` on a generation mismatch (`viewport.generation !=
/// scratch.last_viewport_generation`), independent of any `Changed` filter, so
/// `ui_layout_apply` always performs the real walk this exercises under Miri.
fn run_pair(world: &mut EcsMaster) {
    {
        let vp = world.resource_mut::<UiViewport>();
        vp.generation = vp.generation.wrapping_add(1);
    }
    world.run_system(ui_layout_discovery);
    world.run_system(ui_layout_apply);
}

#[test]
fn miri_nested_layout_walk_is_sound() {
    let mut world = EcsMaster::new();
    world.insert_resource(UiViewport { width: 200.0, height: 300.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());

    // A small 3-level tree to exercise the recursive get_component_mut walk +
    // the mem::take scratch protocol across depths.
    let root = spawn(&mut world, col(px(200.0), px(300.0)), None, true);
    let a = spawn(&mut world, col(px(100.0), px(60.0)), Some(root), false);
    let _a0 = spawn(&mut world, col(px(40.0), px(20.0)), Some(a), false);
    let _a1 = spawn(&mut world, col(px(40.0), px(20.0)), Some(a), false);
    let _b = spawn(&mut world, col(px(100.0), Unit::Stretch(1.0)), Some(root), false);

    run_pair(&mut world);

    // The walk wrote finite rects (the aliasing-sound result).
    let ra = world.get_component::<ComputedRect>(a).expect("rect a");
    assert!(ra.h.is_finite() && ra.w.is_finite(), "rect finite after sound walk");
    assert_eq!(ra.h, 60.0, "child a laid out");

    // A second pass (steady state) must also be sound: mem::take/put-back round
    // trip with the now-populated scratch.
    run_pair(&mut world);
    let ra2 = world.get_component::<ComputedRect>(a).expect("rect a 2");
    assert_eq!(ra2.h, 60.0, "stable across a second pass");
}

#[test]
fn miri_set_if_changed_write_is_sound() {
    let mut world = EcsMaster::new();
    world.insert_resource(UiViewport { width: 100.0, height: 100.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());

    let root = spawn(&mut world, col(px(100.0), px(100.0)), None, true);
    let child = spawn(&mut world, col(px(50.0), px(50.0)), Some(root), false);
    run_pair(&mut world);
    assert_eq!(world.get_component::<ComputedRect>(child).expect("rect").h, 50.0);

    // Re-run: the shared-read compare + (suppressed) Mut guard path under Miri.
    run_pair(&mut world);
    assert_eq!(world.get_component::<ComputedRect>(child).expect("rect").h, 50.0);
}

#[test]
fn miri_empty_root_walk_is_sound() {
    // No children: exercises the leaf path + empty scratch pools.
    let mut world = EcsMaster::new();
    world.insert_resource(UiViewport { width: 50.0, height: 50.0, scale_factor: 1.0, generation: 0 });
    world.insert_resource(LayoutScratch::with_seeds());
    let root = spawn(&mut world, col(Unit::Auto, Unit::Auto), None, true);
    run_pair(&mut world);
    let r = world.get_component::<ComputedRect>(root).expect("root rect");
    assert!(r.w.is_finite() && r.h.is_finite(), "empty root rect finite");
}
