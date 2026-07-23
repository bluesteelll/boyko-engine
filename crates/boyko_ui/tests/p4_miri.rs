//! GATE 6 — MIRI (Tree-Borrows soundness) over the P4 unsafe surface.
//!
//! This test ACTUALLY exercises the binding trampoline end-to-end — NOT a subset
//! that skips the unsafe (the gap that bit P3):
//!
//!   * `get_component_raw(source, comp)` returns a live `*const u8` row, which is
//!     passed THROUGH the installed `BindAccessor` fn-pointer trampoline
//!     (`(acc.fmt)(row, field, &mut dyn Write)` / `(acc.value)(row, field)`),
//!     whose generated `fmt_erased`/`value_erased` bodies do the
//!     `&*(p as *const Health)` reborrow + read of a real bound component. The
//!     bind apply path drives exactly this.
//!   * `get_component_changed_tick` (the read-only entity-keyed tick reader with
//!     its `addr_of!((*archetype_ptr).component_pools)` projection) and
//!     `any_changed_since` (the `&Archetype` epoch scan) are driven by the bind
//!     discovery/apply.
//!   * the interaction path: `ui_focus_system` (hit-test, set-if-changed
//!     `Interaction`/`RelativeCursorPosition`, EnableTag toggles, the
//!     `mem::take` scratch protocol) and `ui_dispatch_system` (the
//!     `get_component_raw(origin, OnClick)` re-validation + `ui_press`).
//!
//! Small fixtures (few entities, few frames) so Miri stays tractable.
//!
//! Run (windows-gnu nightly, Tree-Borrows via .cargo/config):
//!   RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!     cargo miri test -p boyko-ui --test p4_miri

// Test-harness plumbing only: `Arc<Mutex<…>>` is this repo's established probe for
// smuggling a spawned `Entity` / a `UiParseReport` out of the `Send + Sync` one-shot
// system closure, and a file-static `Mutex<()>` serializes tests that arm a process-global
// (the counting allocator, the watch-poll counters). Not engine code — the whole file is
// compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_macros::{Bindable, Component};

use boyko_input::{ActionKind, ActionState, Actionlike, MouseButton, PhysicalInput};

use boyko_ui::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use boyko_ui::binding::components::{BindText, BindValue, TemplateId, UiTextBuffer, UiValue};
use boyko_ui::binding::Bindable;
use boyko_ui::components::{ComputedRect, UiRoot};
use boyko_ui::interaction::action::OnClick;
use boyko_ui::interaction::components::{Interaction, RelativeCursorPosition};
use boyko_ui::interaction::dispatch::ui_dispatch_system;
use boyko_ui::interaction::focus::{
    ui_focus_system, UiInputFocus, UiInteractionConfig, UiInteractionScratch, UiPointerState,
};
use boyko_ui::interaction::plugin::{TAG_UI_FOCUSED, TAG_UI_HOVERED, TAG_UI_PRESSED};
use boyko_ui::resources::UiViewport;

/// The bindable source under Miri. `#[derive(Bindable)]` generates the
/// `fmt_erased`/`value_erased` trampolines whose `unsafe { &*(p as *const Health) }`
/// reborrow is the precise UB candidate this test must cover.
#[derive(Component, Bindable, Clone, Copy, Debug)]
#[repr(C)]
struct Health {
    current: f32,
    max: f32,
}

/// Minimal action enum for the dispatch re-validation path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MiriAction {
    Click,
}
impl Actionlike for MiriAction {
    const COUNT: usize = 1;
    fn index(self) -> usize {
        0
    }
    fn from_index(i: usize) -> Option<Self> {
        (i == 0).then_some(MiriAction::Click)
    }
    fn kind(self) -> ActionKind {
        ActionKind::Button
    }
    fn name(self) -> &'static str {
        "Click"
    }
}

fn spawn<F>(world: &mut EcsMaster, f: F) -> Entity
where
    F: FnOnce(&mut Commands) -> Entity + Send + Sync + 'static,
{
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    let f = Mutex::new(Some(f));
    world.run_system(move |mut cmds: Commands| {
        let f = f.lock().unwrap().take().unwrap();
        let e = f(&mut cmds);
        *probe.lock().unwrap() = Some(e);
    });
    sink.lock().unwrap().unwrap()
}

#[test]
fn miri_binding_trampoline_text_and_value() {
    // Drives `ui_bind_apply` over a real BindText + BindValue widget so the
    // installed BindAccessor fn-pointers reborrow `*const u8` (from
    // get_component_raw) into `&Health` and read its fields — the trampoline
    // unsafe is genuinely executed (no skip).
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut world = EcsMaster::new();

    let mut scratch = UiBindScratch::default();
    Health::register_bind_accessor();
    scratch.register_bound_id(Health::component_id());
    world.insert_resource(scratch);

    let mut builder = ScheduleBuilder::new(pool);
    let discovery = builder.add_system(ui_bind_discovery).key();
    builder.add_system(ui_bind_apply).after(discovery);
    let mut schedule = builder.build(&mut world);

    // Warm the tick window past `Tick::ZERO` before spawning the source (a
    // tick-0 spawn would be masked by the ZERO sentinel; see p4_bind.rs).
    schedule.run(&mut world);
    schedule.run(&mut world);

    let comp = Health::component_id();
    let src = spawn(&mut world, move |cmds| cmds.spawn(Health { current: 30.0, max: 80.0 }).id());
    let text = spawn(&mut world, move |cmds| {
        let mut ec = cmds.spawn(BindText {
            source: src,
            comp,
            field: 0,
            field2: 1,
            template: TemplateId::Ratio,
        });
        ec.insert(UiTextBuffer::default());
        ec.id()
    });
    let value = spawn(&mut world, move |cmds| {
        let mut ec = cmds.spawn(BindValue {
            source: src,
            comp,
            num_field: 0,
            den_field: 1,
        });
        ec.insert(UiValue::default());
        ec.id()
    });

    schedule.run(&mut world);
    // The trampoline ran: assert the read values are correct (the fmt/value
    // accessors actually dereferenced the live row).
    assert_eq!(
        world.get_component::<UiTextBuffer>(text).map(|b| b.as_str().to_string()),
        Some("30/80".to_string()),
        "text trampoline formatted current/max"
    );
    let v = world.get_component::<UiValue>(value).map(|v| v.0).unwrap();
    assert!((v - 0.375).abs() < 1e-6, "value trampoline computed 30/80 = 0.375, got {v}");

    // Read-with-tick accessor on the source (the addr_of! projection path).
    assert!(
        world.get_component_changed_tick(src, comp).is_some(),
        "get_component_changed_tick resolves a live source row"
    );
    // None on a dead entity (the prologue's null/gen check).
    let dead = spawn(&mut world, move |cmds| cmds.spawn(Health { current: 1.0, max: 1.0 }).id());
    world.run_system(move |mut cmds: Commands| {
        cmds.entity(dead).despawn();
    });
    assert!(
        world.get_component_changed_tick(dead, comp).is_none(),
        "get_component_changed_tick returns None for a despawned entity"
    );
}

#[test]
fn miri_interaction_focus_dispatch_click_path() {
    // Drives the full focus + dispatch interaction unsafe surface (scratch
    // mem::take protocol, set-if-changed writes, EnableTag toggles,
    // get_component_raw re-validation, ui_press).
    let mut world = EcsMaster::new();
    let hovered_tag = world.register_enable_tag(TAG_UI_HOVERED);
    let pressed_tag = world.register_enable_tag(TAG_UI_PRESSED);
    let focused_tag = world.register_enable_tag(TAG_UI_FOCUSED);
    world.insert_resource(UiInteractionConfig {
        hovered_tag,
        pressed_tag,
        focused_tag,
    });
    world.insert_resource(UiPointerState::default());
    world.insert_resource(UiInputFocus::default());
    world.insert_resource(UiInteractionScratch::default());
    world.insert_resource(UiViewport {
        width: 200.0,
        height: 200.0,
        scale_factor: 1.0,
        generation: 0,
    });
    world.insert_resource(PhysicalInput::default());
    world.insert_resource(ActionState::<MiriAction>::new());

    let btn = spawn(&mut world, |cmds| {
        let mut ec = cmds.spawn(Interaction::None);
        ec.insert(ComputedRect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        ec.insert(UiRoot);
        ec.insert(OnClick(0));
        ec.insert(RelativeCursorPosition::default());
        ec.id()
    });

    let left = MouseButton::Left.dense_index().map(|i| 1u8 << i).unwrap_or(1);

    // Frame 1: press inside.
    {
        let p = world.resource_mut::<PhysicalInput>();
        p.cursor_pos = [50.0, 50.0];
        p.mouse_just_pressed = left;
        p.mouse_pressed = left;
        p.mouse_just_released = 0;
    }
    ui_focus_system(&mut world);
    ui_dispatch_system::<MiriAction>(&mut world);
    assert_eq!(world.get_component::<Interaction>(btn).copied(), Some(Interaction::Pressed));

    // Frame 2: release inside same → click fires (get_component_raw re-validate).
    {
        let p = world.resource_mut::<PhysicalInput>();
        p.mouse_just_pressed = 0;
        p.mouse_pressed = 0;
        p.mouse_just_released = left;
    }
    ui_focus_system(&mut world);
    ui_dispatch_system::<MiriAction>(&mut world);
    assert!(
        world.resource::<ActionState<MiriAction>>().just_pressed(MiriAction::Click),
        "click fired through the re-validated dispatch path"
    );

    // Frame 3: blur (cursor leaves) → reset path.
    {
        let p = world.resource_mut::<PhysicalInput>();
        p.cursor_inside = false;
        p.mouse_just_released = 0;
    }
    ui_focus_system(&mut world);
    assert_eq!(
        world.get_component::<Interaction>(btn).copied(),
        Some(Interaction::None),
        "blur reset forces None"
    );
}
