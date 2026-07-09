//! Relation-edge observers — `OnLink<R>` / `OnUnlink<R>` (Decision 5, critic
//! W2/W3).
//!
//! These are ordinary [`Trigger`] types (one monomorphisation per relation `R`),
//! fired on the COMMITTED edge at the apply window. They reuse the Feature-2
//! trigger/observer registry verbatim — keying on the trigger's own `TypeId`
//! id-space gives the flecs `(R, *)` wildcard analogue for free, with NO new
//! `ObserverKind` variant and NO side index (Principle 0).
//!
//! # Where they fire (the committed-edge rule, critic W2)
//!
//! An edge observer fires on the edge that is actually COMMITTED at apply time,
//! AFTER the dangling/validity guard — never from the read-only hook body (which
//! cannot drive the synchronous `trigger` walk; critic W3). Concretely:
//!
//! * [`OnLink<R>`] fires inside
//!   [`LinkCommand::apply`](crate::ecs::core::relationship::LinkCommand) after its
//!   dangling-target guard — so a link that no-ops on a dead target never fires,
//!   and a cloned-subtree relink (which routes through the same `apply`) fires
//!   per re-established edge.
//! * [`OnUnlink<R>`] fires inside
//!   [`UnlinkCommand::apply`](crate::ecs::core::relationship::UnlinkCommand) after
//!   it confirms the source was actually present in the target's reverse
//!   collection — so a spurious unlink (the self-ref / dangling guard path, or a
//!   missing link) never fires.
//!
//! Both `apply` bodies already run under `&mut EcsMaster` at the same apply
//! window the relationship commands drain in, so the synchronous `trigger` call
//! is sound there (it re-enters the audited command drain on a separate
//! allocation; see `command_queue::apply_via_raw_twin` / BUG-P19-TB-1).
//!
//! # 0%-gate
//!
//! The fire is gated behind a cold `EcsMaster::has_edge_observer(tid)` probe
//! (one global-registry read + one entity-store aggregate read), so a world with
//! no edge observers pays ~nothing per committed edge and the synchronous
//! `trigger` machinery is never entered.

use core::marker::PhantomData;

use crate::ecs::core::component::observers::traversal::{PropagationMode, Toward};
use crate::ecs::core::component::observers::trigger::Trigger;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::relationship::Relationship;

/// Built-in trigger fired when an `R` edge is COMMITTED (a new foreign key, or a
/// re-target's new side).
///
/// Targeted at the SOURCE entity of the edge; `target` is the entity the source
/// now points at. Read it from an observer runner registered via
/// [`EcsMaster::observe_on_link`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::observe_on_link).
///
/// One monomorphisation per relation `R`, so two relations get two distinct
/// dense `TriggerId`s (the `(R, *)` analogue). `PROPAGATION` is
/// [`None`](PropagationMode::None) — an edge event never bubbles by itself.
#[repr(C)]
pub struct OnLink<R: Relationship> {
    /// The entity the source now points at (the committed edge's target).
    pub target: Entity,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Relationship> OnLink<R> {
    /// Constructs the event for a committed `source -> target` edge.
    #[inline]
    pub const fn new(target: Entity) -> Self {
        Self { target, _marker: PhantomData }
    }
}

impl<R: Relationship> Trigger for OnLink<R> {
    // An edge event targets the source and does not bubble: target-only.
    const PROPAGATION: PropagationMode = PropagationMode::None;
    // Never consulted (`PROPAGATION == None`); the `R` bubble is the natural
    // placeholder so the associated type is nameable without an extra import.
    type Traversal = Toward<R>;
    // Never consulted (`PROPAGATION == None`).
    type Broadcast = R;
}

/// Built-in trigger fired when an `R` edge is DESTROYED (an explicit remove, a
/// re-target's old side, a despawn of the source, or a non-cascading target
/// teardown).
///
/// Targeted at the SOURCE entity of the (now-broken) edge; `old_target` is the
/// entity the source used to point at. Read it from an observer runner
/// registered via
/// [`EcsMaster::observe_on_unlink`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::observe_on_unlink).
#[repr(C)]
pub struct OnUnlink<R: Relationship> {
    /// The entity the source used to point at (the destroyed edge's target).
    pub old_target: Entity,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Relationship> OnUnlink<R> {
    /// Constructs the event for a destroyed `source -> old_target` edge.
    #[inline]
    pub const fn new(old_target: Entity) -> Self {
        Self { old_target, _marker: PhantomData }
    }
}

impl<R: Relationship> Trigger for OnUnlink<R> {
    const PROPAGATION: PropagationMode = PropagationMode::None;
    type Traversal = Toward<R>;
    type Broadcast = R;
}
