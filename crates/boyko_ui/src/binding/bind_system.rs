//! `ui_bind_discovery` + `ui_bind_apply` — the change-gated data-bind systems
//! (GUI P4 Decisions 4/5/6/7/8).
//!
//! Split discovery/apply, mirroring `ui_layout_discovery`/`ui_layout_apply`:
//!
//! * [`ui_bind_discovery`] (cheap; sets `dirty`): probes the registered bound
//!   `ComponentId` set via `EcsMaster::any_changed_since` (the `.ui`-dynamic
//!   outer 0%-gate, Decision 6). A still frame finds no changed column and leaves
//!   `dirty == false`, so apply early-returns. Static `ui!`-bound types can be
//!   added to the gate via a host-composed `Query<(), Or<(Changed<C1>, …)>>`
//!   probe that ORs into the same `dirty` flag.
//! * [`ui_bind_apply`] (exclusive; runs only when `dirty`): iterates bound
//!   widgets WIDGET-side, gates each on the source's `changed_tick` via
//!   `get_component_changed_tick` (Decision 5 — read-only, never bumps the
//!   source tick), formats through the `BindAccessor` fn-pointer trampoline
//!   (Decision 7), and writes the sink SET-IF-CHANGED.

use std::fmt::Write as _;
use std::mem;

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::get_bind_accessor;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::change_detection::Tick;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Resource;

use crate::binding::components::{BindText, BindValue, TemplateId, UiTextBuffer, UiValue, NO_FIELD};

/// The bind systems' shared state: the registered dynamic bound id set, the
/// per-frame `dirty` gate, and the `(last_run, this_run]` tick window apply uses
/// for the per-widget source-tick compare (Decision 5/6).
#[derive(Resource, Default)]
pub struct UiBindScratch {
    /// The bound `ComponentId`s the `.ui`-dynamic gate probes (registered at bind
    /// time, closed at runtime). Small set; reused, never per-frame allocated.
    pub dynamic_bound_ids: Vec<ComponentId>,
    /// Discovery's per-frame "some bound source changed" flag, consumed (and
    /// cleared) by apply.
    pub dirty: bool,
    /// The tick at the start of the last apply (the `last_run` of the
    /// `(last_run, this_run]` window). Updated by apply each time it runs.
    pub last_run: Tick,
    /// Retained `BindText` widget-query buffer (alloc-free per-frame reuse).
    text_widgets: Vec<Entity>,
    /// Retained `BindValue` widget-query buffer (alloc-free per-frame reuse).
    value_widgets: Vec<Entity>,
    /// Retained archetype-id scratch backing the widget queries.
    arch_ids: Vec<ArchetypeId>,
}

impl UiBindScratch {
    /// Registers `id` as a dynamic bound source id (idempotent). Call at setup
    /// for every `.ui`-bound component type.
    pub fn register_bound_id(&mut self, id: ComponentId) {
        if !self.dynamic_bound_ids.contains(&id) {
            self.dynamic_bound_ids.push(id);
        }
    }
}

/// Discovery: sets `dirty` when ANY registered bound source changed in the
/// `(last_run, this_run]` window (Decision 6). Cheap — `any_changed_since` is
/// bounded to hosting archetypes and short-circuits; a still frame returns
/// `false` after scanning only the hosting archetypes' live rows.
///
/// Exclusive (`&mut EcsMaster`) so it can read the world's archetype change
/// epochs and the scratch ids together; the `this_run` horizon comes from the
/// world's current tick (the same horizon the typed `Changed` filter uses).
//
// `clippy::needless_pass_by_ref_mut`: writes `dirty` on the scratch resource via
// `&mut self` engine methods clippy cannot see through. Mirrors `ui_layout_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_bind_discovery(world: &mut EcsMaster) {
    let this_run = world.current_tick();
    // Borrow the id set BY REFERENCE (no `clone`): `resource` and
    // `any_changed_since` both take `&self`, so the two shared borrows of `world`
    // coexist. Steady-state allocation-free.
    let changed = {
        let scratch = world.resource::<UiBindScratch>();
        if scratch.dynamic_bound_ids.is_empty() {
            false
        } else {
            world.any_changed_since(&scratch.dynamic_bound_ids, scratch.last_run, this_run)
        }
    };
    world.resource_mut::<UiBindScratch>().dirty = changed;
}

/// Apply: pushes each changed bound source field into its widget sink
/// (Decision 5/7/8). Exclusive — reads source entity X, writes sink entity Y.
/// Runs only when `dirty`.
//
// `clippy::needless_pass_by_ref_mut`: writes sinks via `&mut self` engine
// methods clippy cannot see through. Mirrors `ui_layout_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_bind_apply(world: &mut EcsMaster) {
    let (dirty, last_run, this_run) = {
        let scratch = world.resource::<UiBindScratch>();
        (scratch.dirty, scratch.last_run, world.current_tick())
    };
    if !dirty {
        return;
    }

    apply_text_bindings(world, last_run, this_run);
    apply_value_bindings(world, last_run, this_run);

    let scratch = world.resource_mut::<UiBindScratch>();
    scratch.dirty = false;
    scratch.last_run = this_run;
}

/// Applies every [`BindText`] whose source changed since `last_run`.
///
/// Allocation-free: the widget list + archetype-id scratch are taken out of the
/// retained `UiBindScratch` buffers (so the world-mutating loop holds no resource
/// borrow) and put back to preserve capacity.
fn apply_text_bindings(world: &mut EcsMaster, last_run: Tick, this_run: Tick) {
    let (mut widgets, mut arch_ids) = {
        let s = world.resource_mut::<UiBindScratch>();
        (mem::take(&mut s.text_widgets), mem::take(&mut s.arch_ids))
    };
    world.query_entities_buf(
        &[BindText::component_id(), UiTextBuffer::component_id()],
        &mut widgets,
        &mut arch_ids,
    );
    for &widget in &widgets {
        let Some(bind) = world.get_component::<BindText>(widget).copied() else {
            continue;
        };

        // Source resolve + tick gate (Decision 5): None (despawn/stale/hosting-
        // miss) → skip; not-newer → skip. This None-skip is the trampoline SAFETY
        // precondition.
        let Some(tick) = world.get_component_changed_tick(bind.source, bind.comp) else {
            continue;
        };
        if !tick.is_newer_than(last_run, this_run) {
            continue;
        }

        // Format into a STACK scratch buffer via the type-erased accessor
        // (Decision 7), then set-if-changed the sink.
        let Some(acc) = get_bind_accessor(bind.comp.0) else {
            continue;
        };
        let Some(row) = world.get_component_raw(bind.source, bind.comp) else {
            continue;
        };

        let mut buf = UiTextBuffer::default();
        match bind.template {
            TemplateId::Value => {
                let _ = (acc.fmt)(row, bind.field, &mut buf);
            }
            TemplateId::Ratio => {
                let _ = (acc.fmt)(row, bind.field, &mut buf);
                let _ = buf.write_char('/');
                if bind.field2 != NO_FIELD {
                    let _ = (acc.fmt)(row, bind.field2, &mut buf);
                }
            }
        }

        set_text_if_changed(world, widget, buf);
    }
    // Return the retained buffers (capacity preserved for next frame).
    let s = world.resource_mut::<UiBindScratch>();
    s.text_widgets = widgets;
    s.arch_ids = arch_ids;
}

/// Applies every [`BindValue`] whose source changed since `last_run`.
///
/// Allocation-free: same retained-buffer take/restore as `apply_text_bindings`.
fn apply_value_bindings(world: &mut EcsMaster, last_run: Tick, this_run: Tick) {
    let (mut widgets, mut arch_ids) = {
        let s = world.resource_mut::<UiBindScratch>();
        (mem::take(&mut s.value_widgets), mem::take(&mut s.arch_ids))
    };
    world.query_entities_buf(
        &[BindValue::component_id(), UiValue::component_id()],
        &mut widgets,
        &mut arch_ids,
    );
    for &widget in &widgets {
        let Some(bind) = world.get_component::<BindValue>(widget).copied() else {
            continue;
        };

        let Some(tick) = world.get_component_changed_tick(bind.source, bind.comp) else {
            continue;
        };
        if !tick.is_newer_than(last_run, this_run) {
            continue;
        }

        let Some(acc) = get_bind_accessor(bind.comp.0) else {
            continue;
        };
        let Some(row) = world.get_component_raw(bind.source, bind.comp) else {
            continue;
        };

        let num = (acc.value)(row, bind.num_field);
        let value = if bind.den_field == NO_FIELD {
            num
        } else {
            let den = (acc.value)(row, bind.den_field);
            if den != 0.0 { num / den } else { 0.0 }
        };

        set_value_if_changed(world, widget, value);
    }
    let s = world.resource_mut::<UiBindScratch>();
    s.value_widgets = widgets;
    s.arch_ids = arch_ids;
}

/// Writes the text sink set-if-changed (so the sink's `changed_tick` only bumps
/// on a real text change, keeping the P5 `Changed<UiTextBuffer>` gate effective).
fn set_text_if_changed(world: &mut EcsMaster, widget: Entity, buf: UiTextBuffer) {
    match world.get_component::<UiTextBuffer>(widget) {
        Some(cur) if *cur == buf => {}
        Some(_) => {
            if let Some(mut guard) = world.get_component_mut::<UiTextBuffer>(widget) {
                *guard = buf;
            }
        }
        None => {}
    }
}

/// Writes the value sink set-if-changed.
fn set_value_if_changed(world: &mut EcsMaster, widget: Entity, value: f32) {
    match world.get_component::<UiValue>(widget) {
        Some(cur) if cur.0 == value => {}
        Some(_) => {
            if let Some(mut guard) = world.get_component_mut::<UiValue>(widget) {
                guard.0 = value;
            }
        }
        None => {}
    }
}
