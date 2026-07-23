//! [`UiInteractionPlugin`] + [`UiBindingPlugin`] — wire the P4 interaction and
//! data-bind systems into an [`App`] (GUI P4 Decision 10 schedule ordering).
//!
//! # `UiInteractionPlugin<A>`
//!
//! Registers the three interaction EnableTags (`UiHovered`/`UiPressed`/
//! `UiFocused`), inserts the interaction resources, and schedules
//! `ui_focus_system::<A>` then `ui_dispatch_system::<A>` on `CoreSchedule::Main`.
//!
//! ## Schedule ordering (Decision 10)
//!
//! Decision 10 specifies the UI focus/dispatch run "after the device
//! `begin_frame`, before `freeze_fixed_snapshot`" so the existing OR-accumulate
//! carries the UI live edge into `fixed_just_pressed` exactly once. In this
//! engine those two steps are NOT separable — `update_action_state::<A>` does
//! `begin_frame` → re-aggregate → `freeze_fixed_snapshot` in ONE system
//! (`process.rs:57-76`). The plugin therefore schedules, all `.in_set(GameplaySet)`
//! (which runs after `update_action_state`'s `.before_set(GameplaySet)`):
//!
//! 1. `ui_focus_system` — hit-test + interaction writer;
//! 2. `ui_dispatch_system::<A>` (`.after(focus)`) — writes the UI live edge via
//!    `ActionState::ui_press`;
//! 3. `ui_refreeze_fixed_snapshot::<A>` (`.after(dispatch)`) — the SANCTIONED
//!    re-freeze that carries the UI edge into the Fixed schedule.
//!
//! The Main-facing `just_pressed` set sees the UI edge immediately this frame.
//! The re-freeze (step 3) re-runs `freeze_fixed_snapshot`, which OR-accumulates
//! edges (idempotent for the device bits, ORs in the new UI bit) and re-samples
//! levels (a no-op on the same frame); `clear_consumed_fixed_edges` still clears
//! the frozen edge once per consumed fixed batch, so the UI press reaches exactly
//! the one batch that first runs after it (no-miss, no-double-count). `ui_press`
//! itself writes only the live edge + level (Decision 9), so step 3 is the only
//! second touch of the fixed snapshot and it is the explicit re-freeze, not a
//! sticky-edge writer.
//!
//! # `UiBindingPlugin`
//!
//! Inserts [`UiBindScratch`] and schedules `ui_bind_discovery` then
//! `ui_bind_apply` late on `CoreSchedule::Main` (they do not feed actions).

use boyko_ecs::ecs::core::app::{App, CoreSchedule, Plugin};
use boyko_ecs::ecs::core::schedule::system_set::SystemSet;
use boyko_input::{Actionlike, GameplaySet};

use crate::binding::bind_system::{ui_bind_apply, ui_bind_discovery, UiBindScratch};
use crate::binding::Bindable;
use crate::interaction::dispatch::{ui_dispatch_system, ui_refreeze_fixed_snapshot};
use crate::interaction::focus::{
    ui_focus_system, UiInputFocus, UiInteractionConfig, UiInteractionScratch, UiPointerState,
};

/// The interaction EnableTag names (the stable keys; the numeric ids are
/// first-call-order process-unstable and cached in [`UiInteractionConfig`]).
pub const TAG_UI_HOVERED: &str = "boyko_ui::UiHovered";
/// `UiPressed` enable-tag stable name.
pub const TAG_UI_PRESSED: &str = "boyko_ui::UiPressed";
/// `UiFocused` enable-tag stable name.
pub const TAG_UI_FOCUSED: &str = "boyko_ui::UiFocused";

/// Wires the P4 pointer/keyboard interaction systems for the action enum `A`.
///
/// Add it AFTER [`InputPlugin`](boyko_input::InputPlugin) (the UI systems run
/// `.after(update_action_state::<A>)`).
pub struct UiInteractionPlugin<A: Actionlike> {
    _pd: core::marker::PhantomData<fn() -> A>,
}

impl<A: Actionlike> UiInteractionPlugin<A> {
    /// Creates the plugin.
    #[inline]
    pub fn new() -> Self {
        Self {
            _pd: core::marker::PhantomData,
        }
    }
}

impl<A: Actionlike> Default for UiInteractionPlugin<A> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Actionlike> Plugin for UiInteractionPlugin<A> {
    fn build(&self, app: &mut App) {
        // Mint the three interaction EnableTags now (cold) and cache their ids in
        // `UiInteractionConfig` (Decision 1). Registration is idempotent per name.
        let (hovered_tag, pressed_tag, focused_tag) = {
            let world = app.world_mut();
            (
                world.register_enable_tag(TAG_UI_HOVERED),
                world.register_enable_tag(TAG_UI_PRESSED),
                world.register_enable_tag(TAG_UI_FOCUSED),
            )
        };

        app.insert_resource(UiInteractionConfig {
            hovered_tag,
            pressed_tag,
            focused_tag,
        });
        app.insert_resource(UiPointerState::default());
        app.insert_resource(UiInputFocus::default());
        app.insert_resource(UiInteractionScratch::default());

        // Focus → dispatch → re-freeze, IN the gameplay set. `InputPlugin`
        // registers `update_action_state::<A>` `.before_set(GameplaySet)`, so a
        // system `.in_set(GameplaySet)` runs deterministically AFTER the ingest —
        // the after-device-begin_frame ordering Decision 10 requires, without
        // re-registering the ingest. `focus` is pinned before `dispatch`, and the
        // re-freeze is pinned after `dispatch` so the UI live edge written by
        // `ui_press` is OR-accumulated into the fixed snapshot exactly once
        // (Decision 10 — see module docs).
        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            let focus = b.add_system(ui_focus_system).in_set(GameplaySet).key();
            let dispatch = b
                .add_system(ui_dispatch_system::<A>)
                .in_set(GameplaySet)
                .after(focus)
                .key();
            b.add_system(ui_refreeze_fixed_snapshot::<A>)
                .in_set(GameplaySet)
                .after(dispatch);
        });
    }
}

/// Wires the P4 data-bind systems (`ui_bind_discovery` then `ui_bind_apply`)
/// late on `CoreSchedule::Main`.
///
/// # Registering bound source types
///
/// The `.ui`-dynamic bind path resolves a source field through the type-erased
/// [`BindAccessor`] trampoline, which only works once the source type's accessor
/// is installed AND its `ComponentId` is added to the discovery gate. Call
/// [`register_bindable`] for EVERY `#[derive(Bindable)]` source type before the
/// first bind frame:
///
/// ```ignore
/// app.add_plugins(UiBindingPlugin::default());
/// register_bindable::<Health>(&mut app); // installs the accessor + gate id
/// ```
///
/// [`BindAccessor`]: boyko_ecs::ecs::core::component::component_registry::BindAccessor
#[derive(Default)]
pub struct UiBindingPlugin;

/// The [`SystemSet`] the data-bind systems run in (GUI P4/P6a). Exposed so a
/// downstream consumer (the GUI P6a `UiWidgetsPlugin`) can order its bar driver
/// `.after_set(UiBindSet)` — the cross-plugin edge that makes a `BindValue`-driven
/// `UiValue` write visible to the bar the SAME frame (C2).
#[derive(Clone, Copy, Debug)]
pub struct UiBindSet;
impl SystemSet for UiBindSet {}

impl Plugin for UiBindingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiBindScratch::default());
        app.add_systems_cfg_in(CoreSchedule::Main, |b| {
            let discovery = b.add_system(ui_bind_discovery).in_set(UiBindSet).key();
            b.add_system(ui_bind_apply).in_set(UiBindSet).after(discovery);
        });
    }
}

impl UiBindingPlugin {
    /// Creates the plugin.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

/// Registers `C` as a `.ui`-dynamic bound source type: installs its type-erased
/// [`BindAccessor`] into the registry trampoline table AND adds its `ComponentId`
/// to the [`UiBindScratch`] discovery gate (GUI P4 Decision 6/7).
///
/// Call once per source type at setup, AFTER [`UiBindingPlugin`] has inserted
/// [`UiBindScratch`]. Idempotent: the accessor install is write-once and the gate
/// id is deduplicated.
///
/// Without this the bind discovery never probes `C`'s column (so a still gate is
/// permanent) and [`get_bind_accessor`](boyko_ecs::ecs::core::component::component_registry::get_bind_accessor)
/// returns `None` for `C` (so `ui_bind_apply` skips every `C`-sourced widget) —
/// the entire `.ui`-dynamic data-bind path for `C` is unreachable.
///
/// [`BindAccessor`]: boyko_ecs::ecs::core::component::component_registry::BindAccessor
pub fn register_bindable<C: Bindable>(app: &mut App) {
    C::register_bind_accessor();
    let id = <C as boyko_ecs::ecs::core::component::component::Component>::component_id();
    app.world_mut()
        .resource_mut::<UiBindScratch>()
        .register_bound_id(id);
}
