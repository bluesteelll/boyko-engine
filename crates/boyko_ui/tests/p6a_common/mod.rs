//! Shared harness for the GUI P6a widget-library integration tests.
//!
//! Wires the FULL same-frame widget pipeline the host is expected to run, in the
//! documented order (the `widgets.rs` / `lib.rs` ordering contract):
//!
//! ```text
//! ui_bar_discovery -> ui_bar_apply -> ui_layout_discovery -> ui_layout_apply
//! ```
//!
//! The bar systems run BEFORE `ui_layout_discovery` so a fill's `Unit::Pct` change
//! (written by `ui_bar_apply`) is seen by the SAME-frame relayout — exactly the
//! `ui_text_measure_system` precedent. The schedule keeps each system's
//! `(last_run, this_run]` change-detection window across frames, which both the bar
//! discovery (`any_changed_since(UiValue)`) and the layout discovery rely on (a
//! per-call `run_system` would reset the window every frame).
//!
//! Spawning goes through `Commands` so the `ChildOf`/`Children` hooks maintain the
//! reverse collection the layout walk + the bar fill-child lookup read; freshly
//! reserved entity handles are smuggled out of the `Send + Sync` system closure
//! through an `Arc<Mutex<…>>` probe (the established Phase-11/19 pattern).

#![allow(dead_code)]

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
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::Resource;

use boyko_ui::binding::UiValue;
use boyko_ui::components::{
    Bar, BarFill, ComputedRect, UiAnchor, UiBackground, UiGrid, UiLayout, UiRoot,
};
use boyko_ui::layout::{ui_layout_apply, ui_layout_discovery};
use boyko_ui::resources::{LayoutScratch, UiSafeArea, UiViewport};
use boyko_ui::widgets::{ui_bar_apply, ui_bar_discovery, UiBarScratch};

/// A pending `UiValue` mutation, applied by the IN-SCHEDULE [`bar_value_mutator`]
/// ahead of the bar systems — the `p4_bind` `MutQueue` pattern. Driving a value
/// change through an out-of-band `world.run_system` instead lands the write on a
/// tick that collides with the schedule's recorded `last_run` (a HARNESS artifact,
/// not a bar bug); an in-schedule mutate lands inside the next discovery's
/// `(last_run, this_run]` window so the bar driver sees it the same frame.
#[derive(Resource, Default)]
struct BarValueQueue {
    pending: Vec<(Entity, f32)>,
}

/// In-schedule mutator: drains [`BarValueQueue`] and writes each `UiValue` via
/// `get_component_mut` (bumping `Changed<UiValue>` at the schedule's frame tick).
/// Runs BEFORE `ui_bar_discovery`.
#[allow(clippy::needless_pass_by_ref_mut)]
fn bar_value_mutator(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<BarValueQueue>().pending);
    for (e, v) in pending {
        if let Some(mut g) = world.get_component_mut::<UiValue>(e) {
            *g = UiValue(v);
        }
    }
}

/// A test world running the full P6a same-frame pipeline.
pub struct P6a {
    pub world: EcsMaster,
    schedule: Schedule,
}

impl P6a {
    /// Builds a world with the layout + bar resources and the four-system schedule
    /// in the documented order.
    pub fn new(viewport: UiViewport) -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();
        world.insert_resource(viewport);
        world.insert_resource(UiSafeArea::default());
        world.insert_resource(LayoutScratch::with_seeds());
        world.insert_resource(UiBarScratch::default());
        world.insert_resource(BarValueQueue::default());

        let mut builder = ScheduleBuilder::new(pool);
        let mutate = builder.add_system(bar_value_mutator).key();
        let bar_disc = builder.add_system(ui_bar_discovery).after(mutate).key();
        let bar_apply = builder.add_system(ui_bar_apply).after(bar_disc).key();
        let layout_disc = builder.add_system(ui_layout_discovery).after(bar_apply).key();
        builder.add_system(ui_layout_apply).after(layout_disc);
        let mut schedule = builder.build(&mut world);

        // Warm the tick window PAST `Tick::ZERO` before any widget spawns, so a
        // value spawned at the first real frame is seen as changed by the bar
        // discovery's `(last_run, this_run]` window (the `p4_bind` BindWorld
        // precedent — the ZERO boundary masks first-frame Added otherwise).
        schedule.run(&mut world);
        schedule.run(&mut world);

        Self { world, schedule }
    }

    /// 1000x800, scale 1.0.
    pub fn default_world() -> Self {
        Self::new(UiViewport { width: 1000.0, height: 800.0, scale_factor: 1.0, generation: 0 })
    }

    /// Runs one full pipeline frame.
    pub fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Runs the pipeline until the geometry settles after a value/layout change.
    ///
    /// `ui_bar_apply` writes the fill's `Unit::Pct` on the frame it observes the
    /// `UiValue` change, but `ui_layout_discovery` consuming that SAME-FRAME
    /// in-schedule `Changed<UiLayout>` write lands the recomputed `ComputedRect`
    /// one frame later — the ENGINE's standard one-frame change-propagation between
    /// two systems in the same schedule run (verified to be identical for a plain
    /// in-schedule `UiLayout` mutator with no bar involved; it is the documented
    /// `ui_text_measure_system` measure->layout seam, not a widget defect). A real
    /// host renders continuous frames, so the rect always tracks within one frame;
    /// the tests run that extra settle frame explicitly. Two frames are always
    /// enough (bar->Pct on frame 1, layout->rect on frame 2).
    pub fn run_settled(&mut self) {
        self.schedule.run(&mut self.world);
        self.schedule.run(&mut self.world);
    }

    /// Sets the safe-area inset (a host resize signal).
    pub fn set_safe_area(&mut self, s: UiSafeArea) {
        *self.world.resource_mut::<UiSafeArea>() = s;
    }

    /// Spawns via an arbitrary closure, harvesting the returned handle.
    pub fn spawn_with<F>(&mut self, f: F) -> Entity
    where
        F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
    {
        let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
        let probe = Arc::clone(&sink);
        let f = Mutex::new(Some(f));
        self.world.run_system(move |mut cmds: Commands| {
            let f = f.lock().unwrap().take().expect("spawn closure runs once");
            let e = f(&mut cmds);
            *probe.lock().unwrap() = Some(e);
        });
        let e = sink.lock().unwrap().expect("spawned handle");
        assert!(self.world.has_entity(e), "spawned node is live after apply");
        e
    }

    /// Spawns a Bar track (Bar marker + UiBackground + UiValue + a sized
    /// `UiLayout` + `UiRoot` so the layout solver reaches it) with one `BarFill`
    /// child, returning `(track, fill)`. A Row track makes the fill span the WIDTH
    /// (the health-bar shape); a Column track makes it span the HEIGHT. The fill's
    /// main-axis size is driven by `ui_bar_apply` from the track's `UiValue`.
    pub fn spawn_bar(
        &mut self,
        rect: ComputedRect,
        layout: UiLayout,
        initial_value: f32,
        fill_layout: UiLayout,
    ) -> (Entity, Entity) {
        let track = self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(Bar);
            ec.insert(layout);
            ec.insert(rect);
            ec.insert(UiBackground::default());
            ec.insert(UiValue(initial_value));
            ec.insert(UiRoot);
            ec.id()
        });
        let fill = self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BarFill);
            ec.insert(fill_layout);
            ec.insert(ComputedRect::default());
            ec.insert(UiBackground::default());
            ec.set_parent(track);
            ec.id()
        });
        (track, fill)
    }

    /// Enqueues a track's `UiValue` mutation, applied by the in-schedule
    /// [`bar_value_mutator`] on the next [`run`](Self::run) so it lands inside the
    /// bar discovery's change-detection window (the realistic gameplay-mutation
    /// path — see [`BarValueQueue`]).
    pub fn set_value(&mut self, track: Entity, v: f32) {
        self.world.resource_mut::<BarValueQueue>().pending.push((track, v));
    }

    /// Reads a node's `ComputedRect`.
    pub fn rect(&self, e: Entity) -> ComputedRect {
        *self.world.get_component::<ComputedRect>(e).expect("node has a ComputedRect")
    }

    /// Reads a fill node's main-axis `Unit::Pct` percentage (the bar driver's
    /// output), or `None` if the fill's main-axis unit is not a `Pct`.
    pub fn fill_pct(&self, fill: Entity, is_width: bool) -> Option<f32> {
        let l = self.world.get_component::<UiLayout>(fill)?;
        let u = if is_width { l.width } else { l.height };
        match u {
            boyko_ui::units::Unit::Pct(p) => Some(p),
            _ => None,
        }
    }

    /// A node's children as a `Vec` (order unspecified), or `None` if no `Children`.
    pub fn children_of(&self, parent: Entity) -> Option<Vec<Entity>> {
        self.world.get_component::<Children>(parent).map(|c| c.as_slice().to_vec())
    }

    /// A node's `Changed<UiLayout>` tick (read-only probe — the 0%-work gate).
    pub fn layout_changed_tick(&self, e: Entity) -> Option<u32> {
        self.world
            .get_component_changed_tick(e, UiLayout::component_id())
            .map(|t| t.get())
    }

    /// Spawns an anchored ROOT (UiRoot + a definite-size `UiLayout` + UiAnchor),
    /// returning its handle. The root's measured size is `(layout.width,
    /// layout.height)` (both must be definite `Px`), and the anchor resolve pins it.
    pub fn spawn_anchored_root(&mut self, layout: UiLayout, anchor: UiAnchor) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(layout);
            ec.insert(ComputedRect::default());
            ec.insert(UiRoot);
            ec.insert(anchor);
            ec.id()
        })
    }

    /// Spawns a Grid container with `cols`/`rows` and `n` fixed-size relative
    /// children, returning `(grid, children)`. The grid carries a definite size so
    /// the uniform cell extent is well defined.
    pub fn spawn_grid(
        &mut self,
        rect: ComputedRect,
        size: UiLayout,
        grid: UiGrid,
        n: usize,
        child: UiLayout,
    ) -> (Entity, Vec<Entity>) {
        let g = self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(size);
            ec.insert(rect);
            ec.insert(grid);
            ec.insert(UiRoot);
            ec.id()
        });
        let mut kids = Vec::with_capacity(n);
        for _ in 0..n {
            let k = self.spawn_with(move |cmds| {
                let mut ec = cmds.spawn(child);
                ec.insert(ComputedRect::default());
                ec.set_parent(g);
                ec.id()
            });
            kids.push(k);
        }
        (g, kids)
    }
}

// Re-export the Component trait method used by the harness (`component_id`).
use boyko_ecs::ecs::core::component::component::Component;

/// 1e-3 logical-px approximate equality (the P1 harness tolerance).
#[track_caller]
pub fn approx(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 1e-3, "{what}: expected {b}, got {a} (delta {})", (a - b).abs());
}
