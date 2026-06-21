//! `ui_dispatch_system` — lowers an `Interaction` edge into an `ActionState`
//! edge (GUI P4 dispatch algorithm).
//!
//! Per-edge-kind complexity (Decision 2 / dispatch step list):
//! * **Click — O(1)**: read the slot-0 `click_fired = Some((origin, index))`
//!   stamped at press time, RE-VALIDATE the origin is still alive via
//!   `get_component_raw(origin, OnClick::component_id())` (None on
//!   despawn/stale-gen/lost-OnClick → drop silently), then `ui_press(index)`.
//!   Independent of `Changed<Interaction>` on the release frame.
//! * **Hover (None→Hovered) — O(changed rows)**: edge-triggered. `ui_focus_system`
//!   stamps every node that transitioned None→Hovered into
//!   `UiInteractionScratch::hover_entered` at the transition site; dispatch drains
//!   that buffer and fires `OnHover` ONCE per enter. A held hover re-fires nothing
//!   and a still frame iterates an empty buffer (O(changed), not O(all OnHover)).
//! * **Submit — O(1)**: the `pending_submit` action stamped by the focus system
//!   on an Enter edge while focused.
//!
//! Exclusive (`&mut EcsMaster`): it reads `UiPointerState`, re-validates the
//! click origin against the live world, walks the changed-hover set, and writes
//! the `ActionState<A>` resource. No `Box<dyn Fn>` — the only indirection is the
//! resource deref. No allocation on the still-frame path.

use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_input::{ActionState, Actionlike};

use crate::interaction::action::{NO_ACTION, OnClick, OnHover};
use crate::interaction::focus::{UiInteractionScratch, UiPointerState};

/// Lowers the resolved interaction edges into `ActionState<A>` (GUI P4).
///
/// Scheduled INSIDE the input window, AFTER `ui_focus_system` and BEFORE the
/// fixed-snapshot freeze (Decision 10), so `ui_press` writes only the live edge
/// and the existing OR-accumulate carries it into the fixed loop.
//
// `clippy::needless_pass_by_ref_mut`: writes the `ActionState<A>` resource via
// `&mut self` engine methods clippy cannot see through. Mirrors `ui_focus_system`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_dispatch_system<A: Actionlike>(world: &mut EcsMaster) {
    // ── 1. Click — O(1) (stamped at press, re-validated at release) ─────────
    let click = world.resource::<UiPointerState>().slots[0].click_fired;
    if let Some((origin, index)) = click
        && index != NO_ACTION
        && is_origin_alive(world, origin)
    {
        ui_press::<A>(world, index as usize);
    }

    // ── 3. Submit — O(1) ────────────────────────────────────────────────────
    let submit = world.resource::<UiPointerState>().pending_submit;
    if let Some(index) = submit
        && index != NO_ACTION
    {
        ui_press::<A>(world, index as usize);
    }

    // ── 2. Hover (None→Hovered) — O(changed rows), edge-triggered ───────────
    // Drains the hover-enter edges `ui_focus_system` stamped at the transition
    // site (None→Hovered). A held hover stamps nothing, so OnHover fires exactly
    // once per enter; a still frame drains an empty buffer.
    fire_hover_actions::<A>(world);
}

/// Returns whether the click origin is still alive with the matching generation
/// AND still carries `OnClick` (Decision 2 re-validation). `get_component_raw`
/// returns `None` on a null slot OR a generation mismatch OR a hosting-miss.
#[inline]
fn is_origin_alive(world: &EcsMaster, origin: Entity) -> bool {
    world.get_component_raw(origin, OnClick::component_id()).is_some()
}

/// Fires `OnHover` for each node that entered `Hovered` this frame (the
/// None→Hovered edge stamped by `ui_focus_system` into
/// `UiInteractionScratch::hover_entered`). Edge-triggered + O(changed rows):
/// a held hover stamps nothing, a still frame drains an empty buffer.
fn fire_hover_actions<A: Actionlike>(world: &mut EcsMaster) {
    // Take the edge buffer out so `ui_press` (which borrows `ActionState` mutably)
    // does not conflict with a held scratch borrow; the emptied buffer is put back
    // to retain its capacity (alloc-free across frames).
    let entered = mem::take(&mut world.resource_mut::<UiInteractionScratch>().hover_entered);
    for &entity in &entered {
        // The node may have despawned between focus and dispatch; the OnClick-less
        // node simply has no OnHover to read, so this is a silent skip.
        let Some(index) = world.get_component::<OnHover>(entity).map(|h| h.0) else {
            continue;
        };
        if index != NO_ACTION {
            ui_press::<A>(world, index as usize);
        }
    }
    // Restore the (now-iterated) buffer; `write_interactions` clears it next frame.
    let mut buf = entered;
    buf.clear();
    world.resource_mut::<UiInteractionScratch>().hover_entered = buf;
}

/// Writes a UI-source rising edge for `index` into `ActionState<A>`.
#[inline]
fn ui_press<A: Actionlike>(world: &mut EcsMaster, index: usize) {
    world.resource_mut::<ActionState<A>>().ui_press(index);
}

/// Re-freezes the fixed snapshot AFTER `ui_dispatch_system`, carrying the UI
/// live edge into the Fixed schedule (GUI P4 Decision 10 — the sanctioned
/// re-freeze).
///
/// `update_action_state::<A>` does begin_frame → re-aggregate →
/// `freeze_fixed_snapshot` in ONE inseparable system, so a UI edge written by
/// dispatch (which runs `.after` it) lands AFTER that frame's freeze and would be
/// cleared by next frame's begin_frame before any substep saw it. Decision 10
/// requires the UI edge to reach the fixed loop exactly once; re-running the
/// freeze here closes the window without splitting the device update.
///
/// No-miss / no-double-count: `freeze_fixed_snapshot` OR-accumulates edges (so
/// re-freezing is idempotent for the device bits already frozen and merely ORs in
/// the new UI bit) and overwrites levels (re-sampling the same frame is a no-op).
/// `clear_consumed_fixed_edges` still clears the accumulated frozen edges exactly
/// once per consumed fixed batch, so the UI press is observed by precisely the one
/// batch that first runs after it.
//
// `clippy::needless_pass_by_ref_mut`: writes the `ActionState<A>` resource via
// `&mut self` engine methods clippy cannot see through.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_refreeze_fixed_snapshot<A: Actionlike>(world: &mut EcsMaster) {
    world.resource_mut::<ActionState<A>>().freeze_fixed_snapshot();
}
