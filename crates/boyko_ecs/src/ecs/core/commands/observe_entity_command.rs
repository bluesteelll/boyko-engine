//! `ObserveEntityCommand<E>` — deferred "attach an entity-targeted custom-trigger
//! observer" (Feature 2, Step 6).
//!
//! Constructed by
//! [`EntityCommands::observe`](crate::ecs::core::system::params::entity_commands::EntityCommands::observe).
//! On apply it calls
//! [`EcsMaster::observe_entity_event`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::observe_entity_event),
//! which registers the observer and raises the entity's sticky archetype bit —
//! the same path the direct API takes, just deferred to the command drain.

#![allow(dead_code)]

use core::marker::PhantomData;

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::component::observers::trigger::{Trigger, TriggerFn};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred "attach an entity-targeted observer for custom trigger `E`".
///
/// # Layout
///
/// ```text
/// +0  : entity: Entity         (16 B)
/// +16 : runner: TriggerFn      (8 B fn-ptr)
/// +24 : _marker: PhantomData   (0 B ZST)
/// ```
#[repr(C)]
pub(crate) struct ObserveEntityCommand<E: Trigger> {
    pub(crate) entity: Entity,
    pub(crate) runner: TriggerFn,
    /// `fn() -> E` (NOT `E`) so the command is unconditionally `Send` even
    /// though `Trigger` does not require `Send` (FIX O3): a `fn`-returning
    /// `PhantomData` is `Send + Sync` for any `E`.
    pub(crate) _marker: PhantomData<fn() -> E>,
}

impl<E: Trigger> ObserveEntityCommand<E> {
    /// Crate-internal constructor used by `EntityCommands::observe`.
    #[inline]
    pub(crate) const fn new(entity: Entity, runner: TriggerFn) -> Self {
        Self { entity, runner, _marker: PhantomData }
    }
}

impl<E: Trigger> Command for ObserveEntityCommand<E> {
    #[inline]
    fn apply(self, world: &mut EcsMaster) {
        world.observe_entity_event::<E>(self.entity, self.runner);
    }
}
