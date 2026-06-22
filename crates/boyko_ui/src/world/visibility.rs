//! World-anchor show/hide (GUI P7a) — the hover-driven visibility mechanism.
//!
//! [`ui_world_visibility_system`] reads [`HoveredWorldEntity`] (the resource the
//! FUTURE P7b GPU cursor-ray pick populates; in P7a set directly) and drives the
//! [`UiWorldHidden`] EnableTag on the world-anchor roots: the root tracking the
//! hovered SCENE entity is SHOWN (its hide bit cleared), every other
//! `EntityAnchor` root is HIDDEN (bit set). The toggle is O(1) per root; the
//! enumeration is O(world-anchor roots), only when the hovered value CHANGES (the
//! Changed-gate — a still hover input does no work).
//!
//! This is NOT the pick itself (that is P7b's GPU depth/SDF cursor ray). It is
//! the headless-testable show/hide path: a test sets `HoveredWorldEntity`
//! directly and asserts the right root's bit.
//!
//! # Two independent visibility authorities (M4)
//!
//! [`UiWorldHidden`] (hover-driven, this system, direct `&mut` toggle) is
//! DISTINCT from [`UiWorldCulled`](super::components::UiWorldCulled) (frustum,
//! the project system, deferred-or-direct toggle). The layout pass skips a root
//! with EITHER bit set, so the two never race a shared bit.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Resource;

use crate::world::components::{HoveredWorldEntity, UiWorldAnchor, UiWorldHidden, WorldTarget};
use boyko_ecs::ecs::core::component::component::Component;

/// Private tracked previous-hover state — the Changed-gate for
/// [`ui_world_visibility_system`]. A `Resource` (engine storage). `dirty_init`
/// forces the first run to act even when the first hovered value equals the
/// default `None`.
#[derive(Resource, Clone, Copy, Debug)]
pub struct UiWorldHoverState {
    /// The hovered entity acted on last run.
    prev: HoveredWorldEntity,
    /// `false` until the first run, so the initial frame always applies (hides
    /// every `EntityAnchor` root) instead of short-circuiting on `prev == cur`.
    initialized: bool,
}

impl Default for UiWorldHoverState {
    #[inline]
    fn default() -> Self {
        Self {
            prev: HoveredWorldEntity(None),
            initialized: false,
        }
    }
}

/// Drives [`UiWorldHidden`] from [`HoveredWorldEntity`] (GUI P7a show/hide).
///
/// An EXCLUSIVE system (`&mut EcsMaster`): the EnableTag mutators are
/// `&mut self`, and resolving "the root tracking entity `e`" needs entity
/// enumeration (no entity-yielding `QueryData`). Runs in the single-threaded
/// apply window.
///
/// Algorithm:
/// 1. Read [`HoveredWorldEntity`] + the tracked `prev`. If UNCHANGED (and already
///    initialized), return — the still-input 0%-overhead path.
/// 2. For every world-anchor root that tracks a SCENE entity
///    ([`WorldTarget::EntityAnchor`]): clear [`UiWorldHidden`] (show) iff it
///    tracks the hovered entity, else set it (hide). A fixed [`WorldTarget::WorldPos`]
///    root is NOT hover-driven and is left untouched.
/// 3. Store the current hover into `prev`.
///
/// `None` hides every `EntityAnchor` root (nothing hovered). The match is
/// O(world-anchor roots) — tens, not entities — and only on a hover CHANGE.
//
// `clippy::needless_pass_by_ref_mut`: `resource` / `query_entities` /
// `get_component` / `enable` / `disable` reach through `&mut self` engine
// methods clippy cannot see through. Mirrors `ui_layout_apply`.
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn ui_world_visibility_system(world: &mut EcsMaster) {
    let current = *world.resource::<HoveredWorldEntity>();
    let state = *world.resource::<UiWorldHoverState>();

    // The Changed-gate: a still hover input does no work.
    if state.initialized && state.prev == current {
        return;
    }

    let hovered = current.0;

    // Enumerate the world-anchor roots and toggle their hide bit. `query_entities`
    // allocates; this is the rare hover-CHANGE path (O(tens) roots), off any
    // per-entity hot loop.
    let roots = world.query_entities(&[UiWorldAnchor::component_id()]);
    for root in roots {
        let Some(anchor) = world.get_component::<UiWorldAnchor>(root).copied() else {
            continue;
        };
        // Only entity-tracking anchors are hover-driven; a fixed WorldPos anchor
        // is left alone (it is not "the hovered object").
        let WorldTarget::EntityAnchor(target) = anchor.target else {
            continue;
        };
        if Some(target) == hovered {
            world.disable::<UiWorldHidden>(root); // show the hovered root
        } else {
            world.enable::<UiWorldHidden>(root); // hide the rest
        }
    }

    let st = world.resource_mut::<UiWorldHoverState>();
    st.prev = current;
    st.initialized = true;
}
