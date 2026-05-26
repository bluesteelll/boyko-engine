//! `DespawnCommand` — deferred per-entity teardown.
//!
//! Phase 11 §6.5 / EC11. Constructed by
//! [`EntityCommands::despawn`](crate::ecs::core::system::params::entity_commands::EntityCommands::despawn)
//! and [`Commands::despawn`](crate::ecs::core::system::params::commands::Commands::despawn);
//! flushed by `CommandQueue::apply` under exclusive `&mut EcsMaster`. Apply
//! delegates to the existing
//! [`EcsMaster::delete_entity`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::delete_entity)
//! path (CR3 + RemoveOutcome bookkeeping).

#![allow(dead_code)]

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred "delete entity" command.
///
/// # Layout (plan §11.5)
///
/// `+0..8` — `entity: Entity` (8 B). 8 B total.
#[repr(C)]
pub struct DespawnCommand {
    /// The entity to delete. Captured at enqueue time. On apply, the
    /// generation guard inside `EcsMaster::delete_entity` rejects stale
    /// handles (W1 / EC11): false return triggers a debug_assert; release
    /// builds silently no-op.
    pub(crate) entity: Entity,
}

impl Command for DespawnCommand {
    fn apply(self, world: &mut EcsMaster) {
        // W1: a stale or never-registered entity yields `false`. We
        // debug_assert + no-op (release) per EC11.
        let ok = world.delete_entity(self.entity);
        debug_assert!(
            ok,
            "DespawnCommand::apply: entity {:?} is stale or never registered \
             (race / use-after-despawn — silent no-op in release per EC11)",
            self.entity
        );
    }
}
