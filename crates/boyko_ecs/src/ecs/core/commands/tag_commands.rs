//! `AddTagCommand` / `RemoveTagCommand` — deferred dynamic-tag attach /
//! detach (Phase 22 D9).
//!
//! Constructed by
//! `EntityCommands::add_tag`
//! /
//! `EntityCommands::remove_tag`;
//! flushed by `CommandQueue::apply` under exclusive `&mut EcsMaster`.
//!
//! Both are POD payloads (an id pair — no type erasure, no bundle bytes) and
//! both delegate to the direct API ([`EcsMaster::add_tag`] /
//! [`EcsMaster::remove_tag`]), mirroring `DespawnCommand → delete_entity`:
//! the direct methods own the hook fire sites and the depth-bracketed drain,
//! which no-ops at depth ≥ 1 here so the outermost drive drains (Q-A1).
//!
//! Dead / stale entities are a SILENT no-op on apply (plan D9) — a despawn
//! may legitimately race an enqueued tag op within the same frame, so unlike
//! `InsertCommand` / `RemoveCommand` there is no debug_assert.

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::component::component_registry::TagId;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred "attach dynamic tag to entity" command (Phase 22 D9).
///
/// # Layout
///
/// ```text
/// +0  : entity: Entity   (16 B — usize id + u32 generation + pad)
/// +16 : tag: TagId       (8 B — repr(transparent) over ComponentId)
/// +24 : end
/// ```
///
/// `#[repr(C)]` for the documented compact layout, matching the other
/// command payloads.
#[repr(C)]
pub(crate) struct AddTagCommand {
    pub(crate) entity: Entity,
    pub(crate) tag: TagId,
}

impl Command for AddTagCommand {
    fn apply(self, world: &mut EcsMaster) {
        // Absent tag ⇒ migrate; present tag ⇒ in-place replace semantics;
        // dead entity ⇒ silent no-op — all owned by the direct API.
        world.add_tag(self.entity, self.tag);
    }
}

/// Deferred "detach dynamic tag from entity" command (Phase 22 D9). Same
/// layout as [`AddTagCommand`].
#[repr(C)]
pub(crate) struct RemoveTagCommand {
    pub(crate) entity: Entity,
    pub(crate) tag: TagId,
}

impl Command for RemoveTagCommand {
    fn apply(self, world: &mut EcsMaster) {
        // Present tag ⇒ migrate (empty archetype when last); absent tag /
        // dead entity ⇒ silent no-op — all owned by the direct API.
        world.remove_tag(self.entity, self.tag);
    }
}
