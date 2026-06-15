//! `EnableTagCommand` — deferred EnableTag toggle (Decision D3 / Step 9).
//!
//! Constructed by
//! [`EntityCommands::enable`](crate::ecs::core::system::params::entity_commands::EntityCommands::enable)
//! / [`EntityCommands::disable`](crate::ecs::core::system::params::entity_commands::EntityCommands::disable)
//! (and their dynamic `_id` variants); flushed by `CommandQueue::apply` under
//! exclusive `&mut EcsMaster`.
//!
//! A single POD payload (an `Entity`, an [`EnableTagId`], and a `bool` value
//! — no type erasure, no bundle bytes) that delegates to the direct API
//! ([`EcsMaster::enable_id`] / [`EcsMaster::disable_id`]), mirroring the
//! Phase-22 `AddTagCommand → EcsMaster::add_tag` precedent: the direct method
//! owns the fire-site / no-op contract.
//!
//! Dead / stale entities are a SILENT no-op on apply (Decision D3) — a despawn
//! may legitimately race an enqueued toggle within the same frame (the
//! `T-INTERLEAVE` hazard), so unlike `InsertCommand` / `RemoveCommand` there
//! is no debug_assert. The bit op resolves the row via the live
//! `inland.unit_index()` at apply time (never a captured enqueue-time row), so
//! a swap-remove that moves another entity into the target row is honored.
//!
//! [`EcsMaster::enable_id`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::enable_id
//! [`EcsMaster::disable_id`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::disable_id

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::component::component_registry::EnableTagId;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred "set / clear an EnableTag bit on an entity" command (Decision D3 /
/// Step 9).
///
/// # Layout
///
/// ```text
/// +0  : entity: Entity        (16 B — usize id + u32 generation + pad)
/// +16 : tag: EnableTagId      (8 B — repr(transparent) over ComponentId)
/// +24 : value: bool           (1 B — true = enable, false = disable)
/// +32 : end (padded)
/// ```
///
/// `#[repr(C)]` for the documented compact layout, matching the other command
/// payloads (byte-arena stored — no `Box<dyn>`).
#[repr(C)]
pub(crate) struct EnableTagCommand {
    pub(crate) entity: Entity,
    pub(crate) tag: EnableTagId,
    pub(crate) value: bool,
}

impl Command for EnableTagCommand {
    fn apply(self, world: &mut EcsMaster) {
        // Dead / stale entity ⇒ silent no-op; the row is resolved via the live
        // inland at apply time — all owned by the direct API (Decision D3).
        if self.value {
            world.enable_id(self.entity, self.tag);
        } else {
            world.disable_id(self.entity, self.tag);
        }
    }
}
