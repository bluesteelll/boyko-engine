//! P2 Test #8 — Miri UB backstop for the `ui!` macro spawn path.
//!
//! The macro emits SAFE code, but the spawn / insert / `ChildOf` path it drives
//! exercises `Bundle::for_each_component_bytes`'s `unsafe` and the hierarchy
//! hooks' deferred-command applies. This confirms the macro feeds well-formed
//! inputs (no aliasing / use-after-free / uninit reads through the byte-blit).
//!
//! Driven WITHOUT the threadpool: each `ui!` invocation runs through
//! `EcsMaster::run_system` on `&mut world` (the pool's worker-join stalls under
//! Miri; the pool itself is validated separately under loom + Miri in Phase 9).
//! Trees are kept small so Miri stays fast.
//!
//! Run (NOTE the `-Zmiri-ignore-leaks`):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu cargo miri test -p boyko-ui \
//!   --test miri_ui_macro
//! ```
//!
//! `-Zmiri-ignore-leaks` is required: spawning entities reaches the by-design
//! bounded `BundleColumnCache` `Box::leak` (SBO6, tracked as #53 — NOT-A-BUG, a
//! deliberate borrow-decoupling leak) plus the retained `Children` `Vec`s on an
//! intentionally-leaked `EcsMaster`. Those allocator leaks are orthogonal to
//! Tree Borrows; the flag isolates the TB (UB) signal, matching the established
//! config of the sibling Miri suites (`miri_layout`, `miri_phase19`, …).

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::Children;
use boyko_ecs::ecs::core::system::Commands;

use boyko_ui::components::{ComputedRect, UiLayout, UiName, UiRoot};
use boyko_ui::prelude::ui;
use boyko_ui::units::Unit;

/// Reads a parent's children by their `UiName` (post-apply harvest).
fn child_named(world: &EcsMaster, parent: Entity, name: &str) -> Entity {
    let kids = world.get_component::<Children>(parent).map(|c| c.as_slice().to_vec()).unwrap_or_default();
    kids.into_iter()
        .find(|&k| world.get_component::<UiName>(k).map(|n| n.as_str() == name).unwrap_or(false))
        .unwrap_or_else(|| panic!("no child named `{name}`"))
}

#[test]
fn miri_ui_leaf_and_bundle_spawn_is_sound() {
    let mut world = EcsMaster::new();

    // A canonical bundle node (UiNodeBundle fast path) + a UiName insert.
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let r = ui! {
            #m_node {
                UiLayout { width: Unit::Px(10.0), ..UiLayout::default() },
                ComputedRect::default()
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let e = sink.lock().unwrap().expect("node");

    assert!(world.has_entity(e), "node live after apply");
    assert_eq!(world.get_component::<UiName>(e).unwrap().as_str(), "m_node");
}

#[test]
fn miri_ui_three_level_tree_links_are_sound() {
    let mut world = EcsMaster::new();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let gp = ui! {
            #m_gp {
                UiLayout::default(),
                UiRoot,
                children: [
                    #m_p {
                        UiLayout::default(),
                        children: [
                            #m_c { UiLayout::default() }
                        ]
                    }
                ]
            }
        };
        *probe.lock().unwrap() = Some(gp);
    });
    let gp = sink.lock().unwrap().expect("gp");
    let p = child_named(&world, gp, "m_p");
    let c = child_named(&world, p, "m_c");

    assert_eq!(world.get_component::<boyko_ecs::ecs::core::hierarchy::ChildOf>(p).unwrap().0, gp);
    assert_eq!(world.get_component::<boyko_ecs::ecs::core::hierarchy::ChildOf>(c).unwrap().0, p);
}
