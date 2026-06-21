//! Shared harness for the P3 (`.ui` text + hot-reload) integration tests.
//!
//! Two capabilities the P3 gates need that the P1 `common` harness does not:
//!
//! * a `.ui`-vs-`ui!` EQUIVALENCE comparator that spawns BOTH paths into the same
//!   world (through `Commands`, one apply window each) and structurally diffs the
//!   resulting entity trees — entity count, per-node component-id SET, per-node
//!   component BYTE values, `ChildOf`/`Children` topology + initial order, and
//!   `UiName` (Gate 1);
//! * a hot-reload driver that writes a temp `.ui` file, points a `UiHotReload`
//!   resource at it, runs the initial `spawn_ui_tree`, and ticks the watch system
//!   over the two-poll settle so a reload reconcile applies in-world (Gate 5/6).
//!
//! Spawning goes through `Commands`; deferred handles are smuggled out of the
//! `Send + Sync` system closure through `Arc<Mutex<…>>` (the Phase-11/19 pattern).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::{ChildOf, Children};
use boyko_ecs::ecs::core::system::Commands;

/// Discovers the document's live root entities WITHOUT touching the crate-private
/// `UiHotReload::doc_roots`: every entity carrying `UiLayout` whose `ChildOf`
/// either is absent or points at a non-UI entity is a root. The P3 test worlds
/// host exactly one document, so this is the document's root set. Roots are
/// returned in ascending entity-id order for determinism.
pub fn discover_ui_roots(world: &EcsMaster) -> Vec<Entity> {
    let all: Vec<Entity> = world.query_entities(&[UiLayout::component_id()]);
    let is_ui = |e: Entity| world.has_component(e, UiLayout::component_id());
    let mut roots: Vec<Entity> = all
        .into_iter()
        .filter(|&e| match world.get_component::<ChildOf>(e) {
            Some(c) => !is_ui(c.0),
            None => true,
        })
        .collect();
    roots.sort_by_key(|e| e.id().0);
    roots
}

use boyko_ui::components::{
    ComputedClip, ComputedRect, ContentSize, StackIndex, UiAbsolute, UiAlign, UiLayout, UiName,
    UiRoot, UiSpacing,
};
use boyko_ui::reload::tree_view::UiTreeView;
use boyko_ui::text::{parse_ui, spawn_ui_tree};
use boyko_ui::UiPlugin;

/// Spawns a `.ui` document into `world` through `Commands` (one apply window) and
/// returns the document's root entity ids in declaration order. Asserts the parse
/// was clean (the equivalence inputs are canonical, well-formed `.ui`).
pub fn spawn_dot_ui(world: &mut EcsMaster, src: &str) -> Vec<Entity> {
    let tree = parse_ui(src);
    assert!(
        tree.report.is_clean(),
        "harness: .ui source for an equivalence/round-trip case must parse clean, got errors: {:?}",
        tree.report.errors
    );
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    // `parse_ui` output must live in the closure; it is `Clone`.
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let roots = spawn_ui_tree(&owned, &mut cmds, &mut report);
        let mut v = probe.lock().expect("probe");
        for r in roots.iter() {
            v.push(r);
        }
    });
    let out = sink.lock().expect("probe").clone();
    for &e in &out {
        assert!(world.has_entity(e), "spawned .ui root is live after apply");
    }
    out
}

/// The set of text-owned component ids that BOTH the `.ui` and `ui!` paths emit,
/// for the per-node SET comparison. `UiSourceOrder` is excluded by construction
/// (it is crate-private and the macro never stamps it — Decision 12); `ChildOf` /
/// `Children` are topology, compared separately.
fn presence_vector(world: &EcsMaster, e: Entity) -> Vec<(&'static str, bool)> {
    vec![
        ("UiLayout", world.has_component(e, UiLayout::component_id())),
        ("ComputedRect", world.has_component(e, ComputedRect::component_id())),
        ("UiSpacing", world.has_component(e, UiSpacing::component_id())),
        ("UiAlign", world.has_component(e, UiAlign::component_id())),
        ("UiAbsolute", world.has_component(e, UiAbsolute::component_id())),
        ("ContentSize", world.has_component(e, ContentSize::component_id())),
        ("StackIndex", world.has_component(e, StackIndex::component_id())),
        ("ComputedClip", world.has_component(e, ComputedClip::component_id())),
        ("UiRoot", world.has_component(e, UiRoot::component_id())),
        ("UiName", world.has_component(e, UiName::component_id())),
    ]
}

/// `Debug`-string projection of a component's bytes (the POD layout components
/// mostly do not derive `PartialEq`; `Debug` is a total deterministic projection
/// of every field). Returns `None` if the component is absent.
fn comp_debug<T: Component + std::fmt::Debug>(world: &EcsMaster, e: Entity) -> Option<String> {
    world.get_component::<T>(e).map(|v| format!("{v:?}"))
}

/// Asserts two nodes carry byte-equal values for every text-owned component that
/// is present on either (presence is asserted by the caller).
#[track_caller]
fn assert_same_values(world: &EcsMaster, a: Entity, b: Entity, what: &str) {
    macro_rules! eqc {
        ($t:ty) => {
            assert_eq!(
                comp_debug::<$t>(world, a),
                comp_debug::<$t>(world, b),
                "{what}: {} byte-value must match between .ui and ui!",
                stringify!($t)
            );
        };
    }
    eqc!(UiLayout);
    eqc!(ComputedRect);
    eqc!(UiSpacing);
    eqc!(UiAlign);
    eqc!(UiAbsolute);
    eqc!(ContentSize);
    eqc!(StackIndex);
    eqc!(ComputedClip);
    eqc!(UiName);
}

/// Reads a node's `UiName` string, if present.
pub fn name_str(world: &EcsMaster, e: Entity) -> Option<String> {
    world.get_component::<UiName>(e).map(|n| n.as_str().to_string())
}

/// Reads a node's parent (`ChildOf` FK), if any.
pub fn parent_of(world: &EcsMaster, e: Entity) -> Option<Entity> {
    world.get_component::<ChildOf>(e).map(|c| c.0)
}

/// Reads a node's children in `Children` slice order (the INITIAL order is the
/// FIFO `add_child` order = declaration order), or empty.
pub fn children_of(world: &EcsMaster, e: Entity) -> Vec<Entity> {
    world
        .get_component::<Children>(e)
        .map(|c| c.as_slice().to_vec())
        .unwrap_or_default()
}

/// Recursively asserts two subtrees rooted at `a` (.ui) and `b` (ui!) are
/// structurally identical: same per-node component SET, same component byte
/// values, same `UiName`, same child count, and same INITIAL child order matched
/// pairwise (the FIFO drain orders both paths' `add_child` by declaration order,
/// so child[i] on the left corresponds to child[i] on the right).
#[track_caller]
pub fn assert_subtree_equiv(world: &EcsMaster, a: Entity, b: Entity, path: &str) {
    // Component-id SET.
    assert_eq!(
        presence_vector(world, a),
        presence_vector(world, b),
        "{path}: per-node component-id SET must match between .ui and ui!"
    );
    // Component byte values.
    assert_same_values(world, a, b, path);

    // Children: same count AND same initial order (pairwise by slot).
    let ca = children_of(world, a);
    let cb = children_of(world, b);
    assert_eq!(
        ca.len(),
        cb.len(),
        "{path}: child COUNT must match (.ui {} vs ui! {})",
        ca.len(),
        cb.len()
    );
    for (i, (&ka, &kb)) in ca.iter().zip(cb.iter()).enumerate() {
        // Topology: each child's ChildOf points back at its own parent.
        assert_eq!(parent_of(world, ka), Some(a), "{path}: .ui child[{i}] ChildOf == parent");
        assert_eq!(parent_of(world, kb), Some(b), "{path}: ui! child[{i}] ChildOf == parent");
        assert_subtree_equiv(world, ka, kb, &format!("{path}/child[{i}]"));
    }
}

/// Counts the entities reachable in a subtree (pre-order), for entity-count
/// equivalence.
pub fn subtree_count(world: &EcsMaster, root: Entity) -> usize {
    let mut n = 1;
    for c in children_of(world, root) {
        n += subtree_count(world, c);
    }
    n
}

// ─────────────────────────── hot-reload driver ────────────────────────────

/// A temp `.ui` file whose path is leaked to `'static` (the `UiHotReload`
/// resource stores `&'static str`). The file is created in the OS temp dir with a
/// unique name. Dropped files are left for the OS to reap (a few KB; cold test).
pub struct TempUi {
    pub path: &'static str,
}

impl TempUi {
    /// Creates a unique temp `.ui` file with the given initial contents.
    pub fn new(tag: &str, contents: &str) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut dir = std::env::temp_dir();
        dir.push(format!("boyko_ui_p3_{tag}_{pid}_{n}.ui"));
        std::fs::write(&dir, contents).expect("write temp .ui");
        let path: &'static str =
            Box::leak(dir.to_string_lossy().into_owned().into_boxed_str());
        Self { path }
    }

    /// Rewrites the file contents. The mtime advances; the watch settle needs the
    /// SAME (mtime,size) twice, so the driver below sleeps past one interval and
    /// ticks twice.
    pub fn write(&self, contents: &str) {
        std::fs::write(self.path, contents).expect("rewrite temp .ui");
    }
}

/// A hot-reload test world driven through the PUBLIC `UiPlugin` + `App` path
/// (the realistic wiring): `finish()` runs the plugin's startup system (initial
/// parse + spawn + `doc_roots` capture + signature seed), `update()` ticks the
/// `ui_hot_reload_system` registered on `CoreSchedule::Main`. The plugin owns the
/// crate-private `UiHotReload` wiring, so the harness needs no private accessors.
///
/// The plugin's poll interval is set to 1 ms so the throttle clears quickly; the
/// two-poll settle still requires the same `(mtime,size)` observed across two
/// ticks, which `reload` provides with short real sleeps between updates.
pub struct ReloadWorld {
    pub app: App,
    pub temp: TempUi,
    pub roots: Vec<Entity>,
}

impl ReloadWorld {
    /// Builds an `App` with a single-threaded pool (deterministic), adds a
    /// `UiPlugin` pointed at a fresh temp `.ui` file (1 ms poll), and `finish`es
    /// it (runs the startup load). The initial document is now spawned and the
    /// watch resource is seeded.
    pub fn new(tag: &str, initial: &str) -> Self {
        let temp = TempUi::new(tag, initial);
        let mut app = App::with_threads(1);
        app.add_plugin(
            UiPlugin::new()
                .with_ui_path(temp.path)
                .with_hot_reload(true)
                .with_poll_interval(Duration::from_millis(1)),
        );
        app.finish(); // runs the startup spawn + signature seed
        let roots = discover_ui_roots(app.world());
        Self { app, temp, roots }
    }

    /// The world borrow.
    pub fn world(&self) -> &EcsMaster {
        self.app.world()
    }

    /// Rewrites the file, then ticks `update()` enough times — with short real
    /// sleeps so mtime advances and the two-poll settle completes — to apply one
    /// reload. The watch sequence is: tick 1 detects the change → records
    /// `pending`; tick 2 confirms the same `(mtime,size)` → reconciles (two-phase
    /// with a drain barrier, all inside the watch system).
    pub fn reload(&mut self, contents: &str) {
        self.temp.write(contents);
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(8));
            self.app.update();
        }
        self.refresh_roots();
    }

    /// Re-discovers the document's roots from the live world (after a reload may
    /// have changed the root set).
    pub fn refresh_roots(&mut self) {
        self.roots = discover_ui_roots(self.app.world());
    }

    /// Ticks `update()` once WITHOUT changing the file (the no-change path).
    pub fn tick(&mut self) {
        self.app.update();
    }

    /// Builds a `UiTreeView` snapshot of the current document subtree (for
    /// inspection in assertions).
    pub fn view(&self) -> UiTreeView {
        let roots = discover_ui_roots(self.app.world());
        UiTreeView::build(self.app.world(), &roots)
    }

    /// Finds a live entity in the document subtree by its `UiName`.
    pub fn find_named(&self, name: &str) -> Option<Entity> {
        let view = self.view();
        view.nodes
            .iter()
            .find(|n| n.name.map(|nm| nm.as_str() == name).unwrap_or(false))
            .map(|n| n.entity)
    }
}
