//! GATE 3 (functional) + GATE 4 — DATA BINDING:
//!   * a data-bound widget updates ONLY when its source component `Changed`;
//!   * on a still frame NO bind work happens AND the bind read does NOT itself
//!     bump `Changed<source>` (the read-only `get_component_changed_tick` /
//!     `get_component_raw` path);
//!   * bound widgets authorable via `ui!` (literal component inserts) and the
//!     direct insert path; the `.ui` `BindText`/`BindValue` form is a documented
//!     deferral (asserted as a recoverable parse error, NOT a silent accept);
//!   * the `ui!` path has no vtable indirection in the SINK (concrete
//!     `UiTextBuffer: fmt::Write`) — the trampoline (`&mut dyn Write`) is the
//!     `.ui`-dynamic path only.
//!
//! Driven through a hand-rolled `Schedule` of `[ui_bind_discovery, ui_bind_apply]`
//! so the change-detection `(last_run, this_run]` tick window advances
//! frame-to-frame (a `run_system` per call would reset it).

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Component, Resource};

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, BindValue, TemplateId, UiTextBuffer, UiValue, NO_FIELD};
use boyko_ui::binding::Bindable;

/// A bindable source component: `current`/`max` (fields 0/1).
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

/// A pending source mutation, applied by the IN-SCHEDULE `mutator_system` ahead
/// of the bind systems. This models a real gameplay system mutating the source
/// within the same schedule (so the write lands on a tick the next bind
/// discovery's `(last_run, this_run]` window covers). Driving a mutation through
/// an out-of-band `world.run_system` instead lands it on the apply-window tick
/// that collides with the schedule's recorded `last_run` — a HARNESS artifact,
/// not a binding bug (an in-schedule mutate re-formats correctly).
#[derive(Resource, Default)]
struct MutQueue {
    pending: Vec<(Entity, Health)>,
}

/// In-schedule mutator: drains `MutQueue` and writes each `Health` via
/// `get_component_mut` (bumping the source `changed_tick` at the schedule's
/// frame tick). Runs BEFORE `ui_bind_discovery`.
#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let pending = std::mem::take(&mut world.resource_mut::<MutQueue>().pending);
    for (e, h) in pending {
        if let Some(mut g) = world.get_component_mut::<Health>(e) {
            *g = h;
        }
    }
}

/// A bind-test world: a `Schedule` of `[mutator, ui_bind_discovery,
/// ui_bind_apply]`, the registered bound-id gate, and the `Health` accessor
/// installed.
struct BindWorld {
    world: EcsMaster,
    schedule: Schedule,
}

impl BindWorld {
    fn new() -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();

        let mut scratch = UiBindScratch::default();
        // Register Health as a dynamic bound source id + install its accessor.
        Health::register_bind_accessor();
        scratch.register_bound_id(Health::component_id());
        world.insert_resource(scratch);
        world.insert_resource(MutQueue::default());

        let mut builder = ScheduleBuilder::new(pool);
        let mutate = builder.add_system(mutator_system).key();
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
        let mut schedule = builder.build(&mut world);

        // Warm the schedule's tick window PAST `Tick::ZERO` before any source is
        // spawned. A source spawned at global tick 0 has `changed_tick == 0`,
        // which `is_newer_than(last_run = 0, this_run)` reports as NOT changed
        // (the `Tick::ZERO` boundary coincides with the schedule's initial
        // `last_run`). In a real `App` the tick has already advanced before
        // systems run + sources spawn, so this models the host ordering; without
        // it the first-frame Added detection is masked by the ZERO sentinel.
        schedule.run(&mut world);
        schedule.run(&mut world);

        Self { world, schedule }
    }

    /// Spawns a `Health` source entity, returning its handle.
    fn spawn_source(&mut self, current: f32, max: f32) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(Health { current, max });
            let _ = &mut ec;
            ec.id()
        })
    }

    /// Spawns a text-bound widget (`BindText` + `UiTextBuffer` sink).
    fn spawn_text_widget(&mut self, source: Entity, template: TemplateId) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BindText {
                source,
                comp: Health::component_id(),
                field: 0,
                field2: if template == TemplateId::Ratio { 1 } else { NO_FIELD },
                template,
            });
            ec.insert(UiTextBuffer::default());
            ec.id()
        })
    }

    /// Spawns a value-bound widget (`BindValue` + `UiValue` sink) binding
    /// `current/max`.
    fn spawn_value_widget(&mut self, source: Entity) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BindValue {
                source,
                comp: Health::component_id(),
                num_field: 0,
                den_field: 1,
            });
            ec.insert(UiValue::default());
            ec.id()
        })
    }

    fn spawn_with<F>(&mut self, f: F) -> Entity
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
        sink.lock().unwrap().expect("spawned handle")
    }

    fn run(&mut self) {
        self.schedule.run(&mut self.world);
    }

    /// Enqueues a `Health` source mutation, applied by the in-schedule
    /// `mutator_system` on the next [`run`](Self::run) (bumps `Changed<Health>` at
    /// the schedule's frame tick — the realistic gameplay-system mutation path).
    fn set_health(&mut self, e: Entity, current: f32, max: f32) {
        self.world
            .resource_mut::<MutQueue>()
            .pending
            .push((e, Health { current, max }));
    }

    fn text_of(&self, e: Entity) -> String {
        self.world
            .get_component::<UiTextBuffer>(e)
            .map(|b| b.as_str().to_string())
            .unwrap_or_default()
    }

    fn value_of(&self, e: Entity) -> f32 {
        self.world.get_component::<UiValue>(e).map(|v| v.0).unwrap_or(f32::NAN)
    }

    /// The source's stored `changed_tick` (read-only probe).
    fn source_changed_tick(&self, e: Entity) -> Option<u32> {
        self.world
            .get_component_changed_tick(e, Health::component_id())
            .map(|t| t.get())
    }
}

// ───────────────────────── update only on Changed ──────────────────────────

#[test]
fn bind_text_updates_on_source_change() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(75.0, 100.0);
    let widget = w.spawn_text_widget(src, TemplateId::Ratio);

    // First frame: the source is Added (a change) → the widget formats.
    w.run();
    assert_eq!(w.text_of(widget), "75/100", "bound text formats current/max on first change");

    // Change the source → re-formats.
    w.set_health(src, 30.0, 100.0);
    w.run();
    assert_eq!(w.text_of(widget), "30/100", "bound text re-formats on source change");
}

#[test]
fn bind_value_updates_ratio_on_source_change() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(50.0, 200.0);
    let widget = w.spawn_value_widget(src);
    w.run();
    assert!((w.value_of(widget) - 0.25).abs() < 1e-6, "value = current/max = 0.25, got {}", w.value_of(widget));

    w.set_health(src, 200.0, 200.0);
    w.run();
    assert!((w.value_of(widget) - 1.0).abs() < 1e-6, "value updated to 1.0, got {}", w.value_of(widget));
}

#[test]
fn bind_value_div_by_zero_guards_to_zero() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(50.0, 0.0);
    let widget = w.spawn_value_widget(src);
    w.run();
    assert_eq!(w.value_of(widget), 0.0, "max==0 → div-by-zero guarded to 0.0");
}

#[test]
fn bind_text_still_frame_does_not_reformat_or_change_sink() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(75.0, 100.0);
    let widget = w.spawn_text_widget(src, TemplateId::Ratio);
    w.run();
    assert_eq!(w.text_of(widget), "75/100", "initial format");

    // Capture the sink's changed_tick after the first apply.
    let sink_tick_before = w
        .world
        .get_component_changed_tick(widget, UiTextBuffer::component_id())
        .map(|t| t.get());

    // A still frame: no source change → no reformat, sink tick UNCHANGED
    // (set-if-changed keeps the sink quiet, so the P5 Changed<UiTextBuffer> gate
    // stays effective).
    for _ in 0..3 {
        w.run();
    }
    let sink_tick_after = w
        .world
        .get_component_changed_tick(widget, UiTextBuffer::component_id())
        .map(|t| t.get());
    assert_eq!(w.text_of(widget), "75/100", "still frames leave the text unchanged");
    assert_eq!(
        sink_tick_before, sink_tick_after,
        "a still frame does not bump the sink's changed_tick (no spurious work)"
    );
}

// ───────────────────────── bind read does NOT bump Changed<source> ──────────

#[test]
fn bind_read_does_not_bump_source_changed_tick() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(75.0, 100.0);
    let _widget = w.spawn_text_widget(src, TemplateId::Value);
    w.run(); // first apply reads the source

    let src_tick_before = w.source_changed_tick(src);

    // Run many still frames: the bind path reads the source via the read-only
    // get_component_changed_tick + get_component_raw, which must NEVER bump the
    // source's changed_tick (that would corrupt the very Changed<source> signal
    // discovery reads).
    for _ in 0..5 {
        w.run();
    }
    let src_tick_after = w.source_changed_tick(src);
    assert_eq!(
        src_tick_before, src_tick_after,
        "reading the bound source does not bump its changed_tick"
    );
}

#[test]
fn bind_unchanged_source_does_not_reapply_after_unrelated_change() {
    // Two sources + two widgets. Changing source B must not cause widget A
    // (bound to source A) to re-format (per-widget tick gate, Decision 5).
    let mut w = BindWorld::new();
    let a = w.spawn_source(10.0, 100.0);
    let b = w.spawn_source(20.0, 100.0);
    let wa = w.spawn_text_widget(a, TemplateId::Value);
    let wb = w.spawn_text_widget(b, TemplateId::Value);
    w.run();
    assert_eq!(w.text_of(wa), "10");
    assert_eq!(w.text_of(wb), "20");

    let wa_sink_before = w
        .world
        .get_component_changed_tick(wa, UiTextBuffer::component_id())
        .map(|t| t.get());

    // Change ONLY b.
    w.set_health(b, 99.0, 100.0);
    w.run();
    assert_eq!(w.text_of(wb), "99", "b's widget updated");
    let wa_sink_after = w
        .world
        .get_component_changed_tick(wa, UiTextBuffer::component_id())
        .map(|t| t.get());
    assert_eq!(
        wa_sink_before, wa_sink_after,
        "a's widget sink tick unchanged when only b's source changed (per-widget gate)"
    );
    assert_eq!(w.text_of(wa), "10", "a's text unchanged");
}

// ───────────────────────── despawned source skipped silently ────────────────

#[test]
fn bind_despawned_source_skips_widget_silently() {
    let mut w = BindWorld::new();
    let src = w.spawn_source(75.0, 100.0);
    let widget = w.spawn_text_widget(src, TemplateId::Value);
    w.run();
    assert_eq!(w.text_of(widget), "75");

    // Despawn the source. The widget's get_component_changed_tick → None → skip.
    w.world.run_system(move |mut cmds: Commands| {
        cmds.entity(src).despawn();
    });
    // Force a dirty frame by registering nothing changed — discovery sees no live
    // changed source, apply early-returns; the widget keeps its last text and does
    // not panic on the dangling source.
    w.run();
    assert_eq!(w.text_of(widget), "75", "despawned source leaves the sink as-is, no panic");
    assert!(!w.world.has_entity(src), "source is despawned");
}

// ───────────────────────── GATE 4: ui! authoring ───────────────────────────

#[test]
fn bind_components_authorable_via_ui_macro_literal_insert() {
    // The `ui!` macro authors BindText/UiTextBuffer as ordinary component
    // literals (no macro change — the plan's "ordinary component literals" path).
    use boyko_ui::prelude::ui;

    let mut w = BindWorld::new();
    let src = w.spawn_source(42.0, 100.0);

    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let comp = Health::component_id();
    w.world.run_system(move |mut cmds: Commands| {
        // A UI node requires a `UiLayout` (the macro's node contract); the bind
        // components ride alongside as ordinary component literals (the plan's
        // "no macro change" authoring path).
        let r = ui! {
            BindText {
                source: src,
                comp,
                field: 0u8,
                field2: NO_FIELD,
                template: TemplateId::Value
            },
            UiTextBuffer::default(),
            boyko_ui::components::UiLayout {
                layout_type: boyko_ui::units::LayoutType::Column,
                ..boyko_ui::components::UiLayout::default()
            }
        };
        *probe.lock().unwrap() = Some(r);
    });
    let widget = sink.lock().unwrap().expect("ui! authored widget");
    assert!(w.world.has_component(widget, BindText::component_id()), "ui! inserted BindText");
    assert!(w.world.has_component(widget, UiTextBuffer::component_id()), "ui! inserted UiTextBuffer");

    w.run();
    assert_eq!(w.text_of(widget), "42", "ui!-authored bound widget formats from its source");
}

// ───────────────────────── GATE 4: .ui authoring (honest deferral) ──────────

#[test]
fn dot_ui_onclick_numeric_authorable() {
    // The `.ui` text format DOES author OnClick as a numeric tuple (Decision 3).
    use boyko_ecs::ecs::core::system::Commands;
    use boyko_ui::interaction::action::OnClick;
    use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport};

    let src = "\
version=1
#btn  UiLayout { layout_type: Column }
    OnClick(3)
";
    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let ent_cell: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let rep_cell: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let ep = Arc::clone(&ent_cell);
    let rp = Arc::clone(&rep_cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let roots = spawn_ui_tree(&owned, &mut cmds, &mut report);
        let mut v = ep.lock().unwrap();
        for r in roots.iter() {
            v.push(r);
        }
        drop(v);
        *rp.lock().unwrap() = report;
    });
    let rep = rep_cell.lock().unwrap().clone();
    assert!(rep.is_clean(), "OnClick(3) authors clean in .ui: {:?}", rep.errors);
    let e = ent_cell.lock().unwrap().first().copied().expect("btn spawned");
    assert_eq!(
        world.get_component::<OnClick>(e).map(|c| c.0),
        Some(3),
        ".ui OnClick(3) lowers to OnClick(3)"
    );
}

#[test]
fn dot_ui_bindtext_is_documented_deferral_recoverable_error() {
    // The `.ui` BindText/BindValue form is a DOCUMENTED P4 deferral: it must be
    // RECOGNIZED (a recoverable per-line error), not silently accepted nor
    // misreported as an unknown component. The deferral error is emitted by
    // `parse_and_insert` at SPAWN time (not by `parse_ui`), so drive the spawn.
    use boyko_ui::text::{parse_ui, spawn_ui_tree, UiParseReport};

    let src = "\
version=1
#hud  UiLayout { layout_type: Column }
    BindText { source: 0, comp: 0, field: 0 }
";
    let tree = parse_ui(src);
    let mut world = EcsMaster::new();
    let rep_cell: Arc<Mutex<UiParseReport>> = Arc::new(Mutex::new(UiParseReport::default()));
    let rp = Arc::clone(&rep_cell);
    let owned = tree.clone();
    world.run_system(move |mut cmds: Commands| {
        let mut report = owned.report.clone();
        let _ = spawn_ui_tree(&owned, &mut cmds, &mut report);
        *rp.lock().unwrap() = report;
    });
    let rep = rep_cell.lock().unwrap().clone();
    assert!(!rep.is_clean(), ".ui BindText is rejected (deferred feature)");
    assert!(
        rep.errors.iter().any(|(_, _, m)| m.contains("deferred") && m.contains("BindText")),
        "the error names BindText as a deferred .ui feature (not 'unknown component'): {:?}",
        rep.errors
    );
}
