//! **Gaia GK-2, end to end: a `boyko_ui` binding whose source is a DENSE
//! component never updates.** RED BY DESIGN; not fixed here.
//!
//! # What this file adds over the kernel-side pin
//!
//! `crates/boyko_ecs/tests/gk2_change_tick_api_dense_blindness.rs` pins the two
//! kernel PRIMITIVES (`EcsMaster::any_changed_since` and
//! `EcsMaster::get_component_changed_tick`) against a hand oracle. This file pins
//! the **claim Gaia's decision line actually makes** — the one a reader of
//! [`docs/gaia/DECISIONS.md`](../../../docs/gaia/DECISIONS.md) §UI bindings item 8
//! is asked to believe:
//!
//! > A bind source on a dense (or bitset) component. `any_changed_since` resolves
//! > per-archetype pools, and non-signature storage owns none, so the gate is
//! > **never true** — the sink never updates and nothing is logged.
//!
//! "The sink never updates" is a statement about the shipped `boyko_ui` bind path,
//! not about a kernel accessor, and it is only demonstrable here — where a real
//! `BindText` widget, a real source entity and the real
//! `[ui_bind_discovery, ui_bind_apply]` schedule are present. A ratified decision
//! line resting on an undemonstrated premise is what this file removes.
//!
//! # The mechanism, confirmed at source (2026-08-30)
//!
//! `crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs`:
//!
//! * `EcsMaster::any_changed_since` — `let Some(pool) =
//!   archetype.component_pools().get_pool(id) else { continue; };`. A
//!   `StorageKind::Dense` id is excluded from every archetype signature and owns
//!   no `ComponentPool`, so every archetype `continue`s and the function returns
//!   `false`.
//! * `EcsMaster::get_component_changed_tick` — `let pool =
//!   pools.get_pool(component_id)?;`. Same resolve, and the `?` turns the miss
//!   into `None`.
//!
//! Both sit between functions that WERE screened: `get_component`,
//! `get_component_mut`, `has_component`, `get_component_raw` and
//! `set_component_raw` each carry a
//! `component_registry::storage_kind(...) == StorageKind::Dense` branch that routes
//! to the `DenseStore`. The two tick readers did not get one. **The ticks they
//! would need exist** — `get_component_raw`'s own doc records that a dense write
//! "bumps the slot's `changed` tick … so a direct-API dense write is observed by a
//! subsequent `Changed<T>` query". This is not "dense has no ticks"; it is "dense
//! has ticks and these two readers do not look".
//!
//! `boyko_ui`'s `binding/bind_system.rs` consumes both, at two INDEPENDENT gates:
//! `ui_bind_discovery` calls `any_changed_since` to set `dirty`, and
//! `apply_text_bindings` / `apply_value_bindings` call
//! `get_component_changed_tick` per widget, where `None` is consumed as
//! "source is gone, skip this bind". Hence the two ignored tests below rather than
//! one: **a fix scoped to either gate alone leaves the binding broken**, and a
//! single end-to-end test would go green on a half fix.
//!
//! # A companion doc defect, recorded here because the fix owes it an edit
//!
//! `any_changed_since`'s doc comment asserts it is "**Bounded to the archetypes
//! that actually host a bound id** (typically 1–few)". For a dense id that is false
//! twice over: the loop visits **every** archetype in the world, and none of them
//! hosts the id. `ui_bind_discovery`'s own doc repeats the claim ("Cheap —
//! `any_changed_since` is bounded to hosting archetypes"). Gaia's decision line
//! already calls for this edit "in the same commit" as the fix.
//!
//! # Why these are `#[ignore]`d rather than fixed
//!
//! GK-2 is a kernel request that [`docs/gaia/CAMPAIGN.md`](../../../docs/gaia/CAMPAIGN.md)
//! gives its own design pass at rung **G8**, and that rung's Gate column says in as
//! many words: *"Nothing here pre-commits GK-2's oracle."* The two candidate
//! remedies — route the tick read through `DenseStore`'s per-slot ticks, or refuse a
//! dense bind source at bake time — are different products, and the second also has
//! to decide what happens to `Bitset` (which owns no ticks at all, so it cannot take
//! the first remedy even if dense does). `boyko_scene`'s dirty scan
//! (`propagation.rs`, `get_component_changed_tick` over `Transform` / `ChildOf`) is
//! a second consumer whose behaviour either remedy changes. Landing a fix here would
//! pre-empt a decision that is not this rung's to make.
//!
//! Run them with:
//! `cargo test -p boyko-ui --test gk2_bind_dense_source_blindness -- --ignored`

// Test-harness plumbing only, exactly as `p4_bind.rs` records it: `Arc<Mutex<…>>`
// is this repo's established probe for smuggling a spawned `Entity` out of the
// `Send + Sync` one-shot system closure. Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Bundle, Component, Resource};

use boyko_ui::binding::Bindable;
use boyko_ui::binding::bind_system::{UiBindScratch, ui_bind_apply, ui_bind_discovery};
use boyko_ui::binding::components::{BindText, TemplateId, UiTextBuffer};

/// The DENSE bind source — owns a global `DenseStore` and NO per-archetype pool.
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct DenseHealth {
    current: f32,
    max: f32,
}

/// The TABLE bind source — the differential control. Every ignored assertion below
/// has a non-ignored twin over this type, so a harness that formats nothing (a wrong
/// tick window, an unregistered accessor, a mis-built schedule) reds the CONTROL
/// first and cannot be mistaken for the GK-2 defect.
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct TableHealth {
    current: f32,
    max: f32,
}

/// A dense component is not itself a `Bundle` (the `Component` derive implements
/// `Bundle` only for the table kind), so the spawn goes through this wrapper.
#[derive(Bundle)]
struct DenseHealthBundle {
    h: DenseHealth,
}

/// Pending source writes, drained by the IN-SCHEDULE mutator ahead of the bind
/// systems — the harness discipline `p4_bind.rs` documents: an out-of-band
/// `run_system` mutation lands on the apply-window tick that collides with the
/// schedule's recorded `last_run`, which is a harness artifact and not a binding
/// bug. Modelling a real gameplay system keeps the window honest.
#[derive(Resource, Default)]
struct MutQueue {
    dense: Vec<(Entity, DenseHealth)>,
    table: Vec<(Entity, TableHealth)>,
}

/// Drains `MutQueue` through `get_component_mut` (which bumps the `changed` tick on
/// EITHER storage kind — the dense arm routes to `DenseStore::insert_or_replace`,
/// whose replace path always stamps the slot).
#[allow(clippy::needless_pass_by_ref_mut)]
fn mutator_system(world: &mut EcsMaster) {
    let (dense, table) = {
        let q = world.resource_mut::<MutQueue>();
        (std::mem::take(&mut q.dense), std::mem::take(&mut q.table))
    };
    for (e, h) in dense {
        if let Some(mut g) = world.get_component_mut::<DenseHealth>(e) {
            *g = h;
        }
    }
    for (e, h) in table {
        if let Some(mut g) = world.get_component_mut::<TableHealth>(e) {
            *g = h;
        }
    }
}

/// Which source ids the discovery gate is told to watch.
#[derive(Clone, Copy, PartialEq)]
enum Watch {
    /// Only the dense id — isolates `any_changed_since` (the discovery gate).
    DenseOnly,
    /// Only the table id — the control.
    TableOnly,
    /// Both, so a TABLE write forces `dirty == true` and the dense-bound widget's
    /// fate is decided purely by `get_component_changed_tick` (the apply gate).
    Both,
}

/// A bind world: `[mutator, ui_bind_discovery, ui_bind_apply]` with the accessors
/// installed and the watched id set registered.
struct BindWorld {
    world: EcsMaster,
    schedule: Schedule,
}

impl BindWorld {
    fn new(watch: Watch) -> Self {
        let pool = ThreadPoolBuilder::new().num_threads(2).build();
        let mut world = EcsMaster::new();

        DenseHealth::register_bind_accessor();
        TableHealth::register_bind_accessor();

        let mut scratch = UiBindScratch::default();
        if matches!(watch, Watch::DenseOnly | Watch::Both) {
            scratch.register_bound_id(DenseHealth::component_id());
        }
        if matches!(watch, Watch::TableOnly | Watch::Both) {
            scratch.register_bound_id(TableHealth::component_id());
        }
        world.insert_resource(scratch);
        world.insert_resource(MutQueue::default());

        let mut builder = ScheduleBuilder::new(pool);
        let mutate = builder.add_system(mutator_system).key();
        let discovery = builder.add_system(ui_bind_discovery).after(mutate).key();
        builder.add_system(ui_bind_apply).after(discovery);
        let mut schedule = builder.build(&mut world);

        // Warm the tick window PAST `Tick::ZERO` before any source is spawned: a
        // source spawned at global tick 0 has `changed_tick == 0`, which
        // `is_newer_than(last_run = 0, …)` reports as NOT changed. `p4_bind.rs`
        // records this as modelling the real `App` ordering.
        schedule.run(&mut world);
        schedule.run(&mut world);

        Self { world, schedule }
    }

    fn spawn_dense_source(&mut self, current: f32, max: f32) -> Entity {
        self.spawn_with(move |cmds| cmds.spawn(DenseHealthBundle { h: DenseHealth { current, max } }).id())
    }

    fn spawn_table_source(&mut self, current: f32, max: f32) -> Entity {
        self.spawn_with(move |cmds| cmds.spawn(TableHealth { current, max }).id())
    }

    /// Spawns a `BindText` widget over `comp` field 0 / field 1 (`Ratio`).
    fn spawn_widget(&mut self, source: Entity, comp: boyko_ecs::ecs::identifiers::primitives::ComponentId) -> Entity {
        self.spawn_with(move |cmds| {
            let mut ec = cmds.spawn(BindText {
                source,
                comp,
                field: 0,
                field2: 1,
                template: TemplateId::Ratio,
            });
            ec.insert(UiTextBuffer::default());
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

    fn set_dense(&mut self, e: Entity, current: f32, max: f32) {
        self.world
            .resource_mut::<MutQueue>()
            .dense
            .push((e, DenseHealth { current, max }));
    }

    fn set_table(&mut self, e: Entity, current: f32, max: f32) {
        self.world
            .resource_mut::<MutQueue>()
            .table
            .push((e, TableHealth { current, max }));
    }

    fn text_of(&self, e: Entity) -> String {
        self.world
            .get_component::<UiTextBuffer>(e)
            .map(|b| b.as_str().to_string())
            .unwrap_or_default()
    }
}

// ───────────────────────────── the control ──────────────────────────────

/// FLOOR. The identical harness over a TABLE source must format and re-format. Not
/// ignored: if this reds, the two ignored tests below are measuring a broken harness
/// rather than the GK-2 defect, and their failure would mean nothing.
#[test]
fn table_source_control_binding_updates() {
    let mut w = BindWorld::new(Watch::TableOnly);
    let src = w.spawn_table_source(75.0, 100.0);
    let widget = w.spawn_widget(src, TableHealth::component_id());

    w.run();
    assert_eq!(
        w.text_of(widget),
        "75/100",
        "control: a TABLE-sourced binding formats on the first change"
    );

    w.set_table(src, 30.0, 100.0);
    w.run();
    assert_eq!(
        w.text_of(widget),
        "30/100",
        "control: a TABLE-sourced binding re-formats on a source change"
    );
}

/// FLOOR. Everything the dense path needs EXCEPT the two tick readers works: the
/// source is a live member, and the value read the trampoline performs
/// (`get_component_raw`) resolves. Not ignored — it isolates the defect to the tick
/// readers, so a later reader cannot mistake the ignored tests for "dense components
/// are not bindable at all".
#[test]
fn dense_source_is_reachable_by_everything_except_the_tick_readers() {
    let mut w = BindWorld::new(Watch::DenseOnly);
    let src = w.spawn_dense_source(75.0, 100.0);

    assert!(
        w.world.has_component(src, DenseHealth::component_id()),
        "control: has_component screens dense and finds the member"
    );
    assert!(
        w.world.get_component::<DenseHealth>(src).is_some(),
        "control: get_component screens dense and reads the member"
    );
    assert!(
        w.world
            .get_component_raw(src, DenseHealth::component_id())
            .is_some(),
        "control: get_component_raw screens dense — this is the row pointer the \
         bind trampoline formats through, so the FORMAT half of the bind path is \
         not what is broken"
    );
}

// ─────────────────────── the two gates, independently ───────────────────────

/// GATE 1 — `ui_bind_discovery`. With only the dense id registered, a write to the
/// dense source must set `dirty` and the sink must format.
///
/// Hand oracle: `"30/100"` after the write (the control test proves the identical
/// harness produces exactly that over a table source). Observed today: the empty
/// string — `any_changed_since` returns `false`, `dirty` stays `false`, and
/// `ui_bind_apply` early-returns without ever looking at the widget.
#[test]
#[ignore = "deferred: Gaia GK-2 owns the fix for the entity-keyed change-tick API \
            over dense/bitset storage (route the read through DenseStore's per-slot \
            ticks vs refuse a dense bind source at bake). Gaia CAMPAIGN rung G8 says \
            'Nothing here pre-commits GK-2's oracle', and boyko_scene's dirty scan is \
            a second consumer either remedy changes. RED BY DESIGN; see this file's \
            header"]
fn discovery_gate_sees_a_dense_bind_source() {
    let mut w = BindWorld::new(Watch::DenseOnly);
    let src = w.spawn_dense_source(75.0, 100.0);
    let widget = w.spawn_widget(src, DenseHealth::component_id());

    w.run();
    w.set_dense(src, 30.0, 100.0);
    w.run();

    assert_eq!(
        w.text_of(widget),
        "30/100",
        "a DENSE-sourced binding must update. The empty string is the GK-2 defect at \
         its FIRST gate: any_changed_since resolves per-archetype pools, a dense id \
         owns none, so the `else {{ continue }}` skips every archetype, `dirty` stays \
         false and ui_bind_apply early-returns. Nothing is logged"
    );
}

/// GATE 2 — the per-widget gate in `apply_text_bindings`, isolated. Both ids are
/// watched and BOTH sources are written, so the TABLE write alone makes
/// `any_changed_since` true and `dirty == true`; `ui_bind_apply` therefore runs and
/// reaches the dense-bound widget. Its fate is now decided purely by
/// `get_component_changed_tick`.
///
/// Hand oracle: `"30/100"`. Observed today: the empty string — the tick read returns
/// `None`, which `apply_text_bindings` consumes as "source despawned, skip this
/// bind".
///
/// **This test is why there are two.** A remedy that fixes only `any_changed_since`
/// makes GATE 1 green and leaves this one red — a binding that is now discovered
/// every frame and still never written, which is worse than either end being broken
/// alone.
#[test]
#[ignore = "deferred: Gaia GK-2 — same design pass as the discovery gate above. This \
            test isolates the SECOND consumer (get_component_changed_tick in \
            apply_text_bindings), which a fix scoped to any_changed_since alone would \
            leave broken. RED BY DESIGN; see this file's header"]
fn apply_gate_resolves_a_dense_bind_source_tick() {
    let mut w = BindWorld::new(Watch::Both);
    let dense_src = w.spawn_dense_source(75.0, 100.0);
    let table_src = w.spawn_table_source(1.0, 1.0);
    let dense_widget = w.spawn_widget(dense_src, DenseHealth::component_id());
    let table_widget = w.spawn_widget(table_src, TableHealth::component_id());

    w.run();
    w.set_dense(dense_src, 30.0, 100.0);
    w.set_table(table_src, 2.0, 4.0);
    w.run();

    // The table widget pins that `dirty` really was true and apply really ran — so a
    // red below cannot be blamed on the discovery gate this test is not measuring.
    assert_eq!(
        w.text_of(table_widget),
        "2/4",
        "precondition: the TABLE write made dirty true and ui_bind_apply ran"
    );
    assert_eq!(
        w.text_of(dense_widget),
        "30/100",
        "with apply demonstrably running, a DENSE-sourced binding must still update. \
         The empty string is the GK-2 defect at its SECOND gate: \
         get_component_changed_tick's `pools.get_pool(component_id)?` returns None for \
         a dense id, and apply_text_bindings reads that None as 'source is gone, skip'"
    );
}
