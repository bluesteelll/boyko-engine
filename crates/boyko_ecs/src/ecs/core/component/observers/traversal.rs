//! `Traversal` — the propagation-hop relation for custom triggers (Feature 2
//! D5).
//!
//! When a custom trigger bubbles, it walks UP a relation one hop at a time. The
//! default relation is [`ChildOfTraversal`] (the `ChildOf` parent pointer); the
//! [`Toward<R>`] bridge lets ANY [`Relationship`] bubble for free (Relations
//! Decision 5). The walk re-derives the relation through the read-only view PER
//! HOP and never holds a `world`-derived `&` across the next fire
//! (OBS-FIRE-LOOP).

use core::marker::PhantomData;

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::ChildOf;
use crate::ecs::core::relationship::Relationship;

/// Computes the next entity a bubbling trigger hops to.
///
/// Implementations read through the read-only [`DeferredEcsMaster`] view, which
/// is re-derived per hop by the trigger walk (no `&` spans the next fire). A
/// `None` return stops the walk.
pub trait Traversal: 'static {
    /// Returns the next hop from `current`, or `None` to stop bubbling.
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity>;
}

/// Bubble toward the target of any single-target [`Relationship`] `R` (Relations
/// Decision 5). A custom trigger bubbles along a non-`ChildOf` relation by
/// setting `type Traversal = Toward<MyRelationship>`.
///
/// Zero cost beyond the existing per-hop `get_component` (one column lookup),
/// already paid by [`ChildOfTraversal`] today.
pub struct Toward<R: Relationship>(PhantomData<R>);

impl<R: Relationship> Traversal for Toward<R> {
    #[inline]
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity> {
        view.get_component::<R>(current).map(|r| r.target())
    }
}

/// The default traversal: bubble up the `ChildOf` parent pointer.
///
/// A standalone struct (rather than `type ChildOfTraversal = Toward<ChildOf>`)
/// so the public name stays a nameable type for the existing observer test
/// suites; the body is the generic `Relationship::target` hop, identical to
/// `Toward<ChildOf>`.
pub struct ChildOfTraversal;

impl Traversal for ChildOfTraversal {
    #[inline]
    fn next(view: &DeferredEcsMaster<'_>, current: Entity) -> Option<Entity> {
        view.get_component::<ChildOf>(current).map(|c| c.target())
    }
}
