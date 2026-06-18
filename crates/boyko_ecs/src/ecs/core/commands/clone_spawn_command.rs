//! `CloneSpawnCommand` — deferred "clone `source` into a pre-reserved entity"
//! command (Feature 3, plan §8).
//!
//! Mirrors `SpawnAtCommand`:
//! `Commands::clone_and_spawn` reserves an [`Entity`] at the callsite (atomic
//! counter) so the user can `.id()` synchronously, then enqueues this command. The
//! actual clone runs at the apply window under `&mut EcsMaster` (structural ops are
//! single-threaded per Phase 9). Apply delegates to
//! `materialize_clone_at`,
//! which writes into the pre-reserved slot (W5: the entity→inland mapping is the
//! LAST step, so a panic mid-clone leaves `entity_master` untouched).

use crate::ecs::core::clone::cloner::EntityCloner;
use crate::ecs::core::clone::deep::clone_subtree_seeded;
use crate::ecs::core::clone::materialize::materialize_clone_at;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred clone-and-spawn command (Feature 3, §8). Carries the pre-reserved
/// destination entity, the source to clone, and the (Copy) cloner config.
#[repr(C)]
pub(crate) struct CloneSpawnCommand {
    /// The pre-reserved destination entity (minted at the `Commands` callsite).
    pub(crate) entity: Entity,
    /// The source to clone (resolved + liveness-checked at apply time).
    pub(crate) source: Entity,
    /// The clone configuration (`EntityCloner` is `Copy` — `ComponentMask` + enums
    /// + bools, no borrows).
    pub(crate) cloner: EntityCloner,
}

// SAFETY (mirrors `SpawnAtCommand` / `InsertCommand`): the payload is plain POD
//   (`Entity` × 2 + the borrow-free `Copy` `EntityCloner`), so moving it across
//   threads is sound. The explicit impls document the `Command: Send + 'static`
//   queue bound.
unsafe impl Send for CloneSpawnCommand {}
unsafe impl Sync for CloneSpawnCommand {}

impl Command for CloneSpawnCommand {
    fn apply(self, world: &mut EcsMaster) {
        // EC8 parity: a stale source is a silent no-op (debug-asserted). The
        // pre-reserved destination id is then left to the counter's monotonic
        // march (one leaked id per missed apply — the same contract as a dropped
        // `SpawnAtCommand`).
        debug_assert!(
            world.has_entity(self.source),
            "CloneSpawnCommand: source entity {:?} is not alive at apply",
            self.source
        );
        if !world.has_entity(self.source) {
            return;
        }

        // Deep vs shallow, into the PRE-RESERVED entity. NO drain here (Q-A1): this
        // runs at depth >= 1 inside the per-system `CommandQueue::apply` bracket;
        // the outermost schedule drive drains after apply returns. On the deep path
        // the ROOT lands in the pre-reserved id (the only user-visible id);
        // descendants get fresh ids inside the seeded walk (Bevy parity:
        // `clone_and_spawn` returns the root).
        if self.cloner.is_deep() {
            clone_subtree_seeded(world, self.source, self.entity, &self.cloner);
        } else {
            materialize_clone_at(world, self.source, self.entity, &self.cloner);
        }
    }
}
