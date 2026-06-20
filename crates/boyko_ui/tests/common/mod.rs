//! Shared harness for the boyko_ui P1 layout integration tests.
//!
//! The layout pair is driven exactly the way a host would drive it: a hand-built
//! [`Schedule`] containing `ui_layout_discovery` then `ui_layout_apply` (pinned
//! with `.after`), run with `schedule.run(&mut world)`. The schedule keeps each
//! system's `(last_run, this_run]` tick window across frames, which is what makes
//! the discovery system's `Changed`/`Added` scan behave correctly frame-to-frame
//! (a fresh `run_system` would reset the window every call — see the Phase-10
//! integration tests for the same reasoning).
//!
//! Spawning goes through `Commands` so the `ChildOf`/`Children` hooks maintain the
//! reverse collection (the layout walk reads `Children`). `Commands::spawn` is
//! deferred (the entity is live only after the apply window), so freshly-spawned
//! handles are smuggled out of the `Send + Sync` system closure through an
//! `Arc<Mutex<…>>` probe — the established Phase-11/Phase-19 pattern.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_ui::components::{
    ComputedRect, ContentSize, UiAbsolute, UiAlign, UiLayout, UiRoot, UiSpacing,
};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiViewport};

/// A full spec for one UI node. `Default` gives a relative Auto×Auto column with
/// a default (`0×0`) `ComputedRect` and no optional components.
#[derive(Clone, Copy, Default)]
pub struct NodeSpec {
    pub layout: UiLayout,
    pub spacing: Option<UiSpacing>,
    pub align: Option<UiAlign>,
    pub absolute: Option<UiAbsolute>,
    pub content: Option<ContentSize>,
    pub root: bool,
}

impl NodeSpec {
    pub fn new(layout: UiLayout) -> Self {
        Self { layout, ..Self::default() }
    }
    pub fn root(layout: UiLayout) -> Self {
        Self { layout, root: true, ..Self::default() }
    }
    pub fn with_spacing(mut self, s: UiSpacing) -> Self {
        self.spacing = Some(s);
        self
    }
    pub fn with_align(mut self, a: UiAlign) -> Self {
        self.align = Some(a);
        self
    }
    pub fn with_absolute(mut self, a: UiAbsolute) -> Self {
        self.absolute = Some(a);
        self
    }
    pub fn with_content(mut self, c: ContentSize) -> Self {
        self.content = Some(c);
        self
    }
}

/// A test world wrapping an [`EcsMaster`], the layout [`Schedule`], and a probe
/// for harvesting freshly-reserved entity handles out of deferred spawns.
pub struct Ui {
    pub world: EcsMaster,
    schedule: Schedule,
}

impl Ui {
    /// Builds a world with `UiViewport` + `LayoutScratch` resources and a schedule
    /// of `[ui_layout_discovery, ui_layout_apply]` (apply pinned after discovery).
    pub fn new(viewport: UiViewport) -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();
        world.insert_resource(viewport);
        world.insert_resource(LayoutScratch::with_seeds());

        let mut builder = ScheduleBuilder::new(pool);
        let discovery = builder.add_system(ui_layout_discovery).key();
        builder.add_system(ui_layout_apply).after(discovery);
        let schedule = builder.build(&mut world);

        Self { world, schedule }
    }

    /// Default 1000×800 viewport.
    pub fn default_world() -> Self {
        Self::new(UiViewport { width: 1000.0, height: 800.0, scale_factor: 1.0, generation: 0 })
    }

    /// Spawns one UI node (always with a default `ComputedRect`), optionally under
    /// `parent`, returning its live handle. Mirrors the requested
    /// `spawn_ui(world, UiLayout, parent)` helper but accepts the richer
    /// [`NodeSpec`] so optional components can be attached in the same apply
    /// window. The entity is live on return (one apply window has run).
    pub fn spawn(&mut self, spec: NodeSpec, parent: Option<Entity>) -> Entity {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        self.world.run_system(move |mut cmds: Commands| {
            // Always: UiLayout + a default ComputedRect (the helper's contract).
            let mut ec = cmds.spawn(spec.layout);
            ec.insert(ComputedRect::default());
            if let Some(s) = spec.spacing {
                ec.insert(s);
            }
            if let Some(a) = spec.align {
                ec.insert(a);
            }
            if let Some(a) = spec.absolute {
                ec.insert(a);
            }
            if let Some(c) = spec.content {
                ec.insert(c);
            }
            if spec.root {
                ec.insert(UiRoot);
            }
            if let Some(p) = parent {
                ec.set_parent(p);
            }
            *probe.lock().expect("probe") = Some(ec.id());
        });
        let e = sink.lock().expect("probe").expect("spawned handle");
        assert!(self.world.has_entity(e), "spawned UI node is live after apply");
        e
    }

    /// Convenience: spawn a root node (inserts `UiRoot`).
    pub fn spawn_root(&mut self, layout: UiLayout) -> Entity {
        self.spawn(NodeSpec::root(layout), None)
    }

    /// Convenience: spawn a relative child under `parent`.
    pub fn spawn_child(&mut self, layout: UiLayout, parent: Entity) -> Entity {
        self.spawn(NodeSpec::new(layout), Some(parent))
    }

    /// Runs one frame of `discovery -> apply`.
    pub fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Reads a node's computed rect (panics if absent — every spawned node carries
    /// one).
    pub fn rect(&self, e: Entity) -> ComputedRect {
        *self
            .world
            .get_component::<ComputedRect>(e)
            .expect("node has a ComputedRect")
    }

    /// Mutates a node's `UiLayout` in place through a command (bumps the
    /// `Changed<UiLayout>` tick), driving an apply window.
    pub fn set_layout(&mut self, e: Entity, layout: UiLayout) {
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(e).insert(layout);
        });
    }

    /// Mutates a node's `ContentSize` (bumps `Changed<ContentSize>`).
    pub fn set_content(&mut self, e: Entity, content: ContentSize) {
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(e).insert(content);
        });
    }

    /// Reparents `child` under `new_parent` (bumps the relevant structural ticks).
    pub fn set_parent(&mut self, child: Entity, new_parent: Entity) {
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(child).set_parent(new_parent);
        });
    }

    /// Despawns `e` (recursive despawn cascade for any subtree).
    pub fn despawn(&mut self, e: Entity) {
        self.world.run_system(move |mut cmds: Commands| {
            cmds.entity(e).despawn();
        });
    }

    /// Bumps the viewport generation (a resize signal) and updates the extent.
    pub fn resize(&mut self, w: f32, h: f32) {
        let vp = self.world.resource_mut::<UiViewport>();
        vp.width = w;
        vp.height = h;
        vp.generation = vp.generation.wrapping_add(1);
    }
}

/// Approximate-equality for layout floats (clamp/percentage arithmetic introduces
/// sub-ULP error). 1e-3 logical px is far below anything observable.
#[track_caller]
pub fn approx(a: f32, b: f32, what: &str) {
    assert!(
        (a - b).abs() < 1e-3,
        "{what}: expected {b}, got {a} (delta {})",
        (a - b).abs()
    );
}

#[track_caller]
pub fn approx_rect(r: ComputedRect, x: f32, y: f32, w: f32, h: f32, what: &str) {
    approx(r.x, x, &format!("{what}.x"));
    approx(r.y, y, &format!("{what}.y"));
    approx(r.w, w, &format!("{what}.w"));
    approx(r.h, h, &format!("{what}.h"));
}
