//! P2 Test #6 — `-D warnings` compile-PASS over representative expansions.
//!
//! The engine gate is `clippy --all-targets -D warnings`. The emitted `ui!` code
//! must be lint-clean: no elided-lifetime path annotation, no needless borrow,
//! no needless/unused `mut`, no unused-variable warning on an unreferenced
//! `#named` handle. This module denies the full warning set so any lint that
//! escapes the expansion fails to compile here (beyond the trybuild suite).

#![deny(warnings)]
#![deny(clippy::all)]

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

mod common;

use std::sync::{Arc, Mutex};

use common::Ui;

use boyko_ecs::ecs::core::entity::entity::Entity;

use boyko_ui::components::{ComputedRect, UiLayout, UiRoot};
use boyko_ui::prelude::ui;
use boyko_ui::units::Unit;

#[test]
fn expansions_compile_clean_under_deny_warnings() {
    let mut ui = Ui::default_world();

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ui.author(move |mut cmds| {
        // 1. A single unnamed leaf.
        let _leaf = ui! {
            UiLayout { width: Unit::Px(1.0), ..UiLayout::default() }
        };

        // 2. A single `#named` leaf whose handle is NEVER referenced — must not
        //    emit `unused_variables` (the macro inserts `let _ = &name;`).
        let _named = ui! {
            #never_used { UiLayout::default() }
        };

        // 3. A node with zero extra inserts (just UiLayout, rect injected).
        let _bare = ui! {
            UiLayout::default()
        };

        // 4. A canonical bundle node (UiLayout + ComputedRect).
        let _bundle = ui! {
            UiLayout::default(),
            ComputedRect::default()
        };

        // 5. A 2-level nest.
        let root = ui! {
            #w6_root {
                UiLayout::default(),
                UiRoot,
                children: [
                    { UiLayout::default() },
                    { UiLayout::default() }
                ]
            }
        };

        *probe.lock().unwrap() = Some(root);
    });

    let root = sink.lock().unwrap().expect("nest root");
    assert!(ui.world.has_entity(root), "expansion produced a live root");
}
