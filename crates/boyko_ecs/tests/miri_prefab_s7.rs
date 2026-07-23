//! Std-lib S7 (REDO, clone-based Prefab) — the soundness oracle for the NEW
//! `prefab` unsafe, on the REAL boyko_ecs path. Authored + run by the orchestrator
//! (the workflow tester never reached this step — transient API outage).
//!
//! Exercises, under Tree Borrows:
//!   * capture's `clone_fn` into the owned `RawBlob` (a `CloneViaFn` slot holds a
//!     LIVE `C`);
//!   * a `TriviallyCopyable` (`Copy`) component's memcpy blob path;
//!   * instantiate re-cloning blob -> fresh rows + the scoped `ChildOf` remap +
//!     `Children` rebuild;
//!   * `Prefab::drop` freeing each blob value exactly once.
//!
//! The DROP-ACCOUNTING assertion is the no-double-free / no-leak gate: every live
//! `DropTracker` (source rows + blob clones + instance rows) is dropped EXACTLY
//! once. A double-free would over-count (and Miri would fault); a leak would
//! under-count.
//!
//! Run under Tree Borrows (the engine's gate; Stacked Borrows over-approximates the
//! command-queue, a documented non-gate — see miri_phase19):
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-ecs --test miri_prefab_s7
//! ```
//! `-Zmiri-ignore-leaks` isolates the TB signal from the `App` -> `ThreadPool`
//! `Arc` teardown leak (an allocator artifact orthogonal to TB), exactly as the
//! kernel's threadpool-bearing Miri suites do.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// Process-global drop counter. This binary holds ONE test so no cross-test race.
static DROPS: AtomicUsize = AtomicUsize::new(0);

/// A non-`Copy` `Clone` component (=> `Cloneability::CloneViaFn`): capture clones it
/// into the blob, instantiate re-clones it, and every value's `Drop` bumps `DROPS`.
#[derive(Component, Clone)]
struct DropTracker(#[allow(dead_code)] u32);

impl Drop for DropTracker {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}

/// A `Copy` component (=> `Cloneability::TriviallyCopyable`): exercises the blob
/// memcpy path (no `clone_fn`, no `drop_fn`).
#[derive(Component, Clone, Copy)]
struct Pod(#[allow(dead_code)] u64);

/// Per-node bundle (a `DropTracker` + a `Pod`), so each captured node has both a
/// `CloneViaFn` and a `TriviallyCopyable` column.
#[derive(Bundle)]
struct NodeBundle {
    tracker: DropTracker,
    pod: Pod,
}

#[test]
fn prefab_clone_capture_instantiate_is_drop_exact_and_remaps() {
    DROPS.store(0, Ordering::Relaxed);

    let mut app = App::new();
    app.finish();

    // Spawn a 2-node `root -> child` tree; each node carries a DropTracker + Pod.
    let sink: Arc<Mutex<Option<(Entity, Entity)>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    app.world_mut().run_system(move |mut cmds: Commands| {
        let root = cmds.spawn(NodeBundle { tracker: DropTracker(1), pod: Pod(10) }).id();
        let child = cmds.spawn(NodeBundle { tracker: DropTracker(2), pod: Pod(20) }).id();
        cmds.entity(root).add_child(child);
        *probe.lock().expect("probe") = Some((root, child));
    });
    let (src_root, src_child) = sink.lock().expect("probe").expect("spawned");
    assert!(app.world().has_entity(src_root) && app.world().has_entity(src_child));

    // CAPTURE: clones each node's DropTracker (CloneViaFn) into the blob (2 live blob
    // clones); Pod is Copy (memcpy, no drop glue). No Drop runs (clone only).
    let prefab = app.world_mut().capture_prefab(src_root);
    assert_eq!(prefab.node_count(), 2, "captured both nodes");
    assert_eq!(DROPS.load(Ordering::Relaxed), 0, "capture clones, drops nothing");

    // INSTANTIATE: re-clones the blob DropTrackers into 2 fresh instance rows; remaps
    // the child's ChildOf to the fresh root + rebuilds Children.
    let inst_root = app.world_mut().instantiate(&prefab);
    assert_eq!(DROPS.load(Ordering::Relaxed), 0, "instantiate clones, drops nothing");

    // Structure: the scoped remap + Children rebuild produced a fresh detached chain.
    assert!(
        app.world().get_component::<ChildOf>(inst_root).is_none(),
        "instance root is detached"
    );
    let kids = app
        .world()
        .get_component::<Children>(inst_root)
        .expect("instance root has Children")
        .as_slice()
        .to_vec();
    assert_eq!(kids.len(), 1, "one fresh child");
    let inst_child = kids[0];
    assert!(inst_child != src_root && inst_child != src_child, "child is fresh");
    assert_eq!(
        app.world().get_component::<ChildOf>(inst_child).map(|c| c.0),
        Some(inst_root),
        "child ChildOf remapped to the FRESH instance root"
    );

    // DROP ACCOUNTING (no double-free / no leak):
    // live DropTrackers = source(2) + blob(2) + instance(2) = 6.
    drop(prefab); // frees the 2 blob clones exactly once.
    assert_eq!(
        DROPS.load(Ordering::Relaxed),
        2,
        "Prefab::drop frees its 2 blob clones exactly once (no double-free)"
    );

    drop(app); // tears down the world: source(2) + instance(2) dropped.
    assert_eq!(
        DROPS.load(Ordering::Relaxed),
        6,
        "all 6 live DropTrackers dropped exactly once (no double-free, no leak)"
    );
}
