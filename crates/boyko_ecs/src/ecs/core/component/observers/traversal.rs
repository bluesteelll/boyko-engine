//! `Traversal` — the propagation-hop relation for custom triggers (Feature 2
//! D5).
//!
//! When a custom trigger bubbles, it walks UP a relation one hop at a time. The
//! default relation is [`ChildOfTraversal`] (the Phase-19 `ChildOf` parent
//! pointer); the trait lets future relations bubble differently. The walk
//! re-derives the relation through the read-only view PER HOP and never holds a
//! `world`-derived `&` across the next fire (OBS-FIRE-LOOP).

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::ChildOf;

/// Computes the next entity a bubbling trigger hops to.
///
/// Implementations read through the read-only [`DeferredEcsMaster`] view, which
/// is re-derived per hop by the trigger walk (no `&` spans the next fire). A
/// `None` return stops the walk.
pub trait Traversal: 'static {
    /// Returns the next hop from `current`, or `None` to stop bubbling.
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity>;
}

/// The default traversal: bubble up the Phase-19 `ChildOf` parent pointer.
pub struct ChildOfTraversal;

impl Traversal for ChildOfTraversal {
    #[inline]
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity> {
        view.get_component::<ChildOf>(current).map(|c| c.0)
    }
}
