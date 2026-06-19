//! `RemoveCommand<C>` — deferred "remove single component `C` from entity".
//!
//! Phase 11 §6.4 / EC10. Constructed by
//! `EntityCommands::remove`.
//! Apply path:
//!
//! 1. Resolve source archetype from the entity's fast inland.
//! 2. Compute target = source \\ {C} via
//!    `without_component_archetype_id`.
//!    Absent C ⇒ silent no-op (W1 — Bevy Issue #10166).
//! 3. Migrate via `migrate_entity_remove`.
//!
//! Bundle-typed remove is deferred to Phase 12 (OQ8).

#![allow(dead_code)]

use core::marker::PhantomData;

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::migration_helpers::{
    migrate_entity_remove, without_component_archetype_id,
};
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, StorageKind};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;

/// Deferred "remove single component `C` from entity" command.
///
/// # Layout (plan §11.4)
///
/// ```text
/// +0  : entity: Entity        (8 B)
/// +8  : _marker: PhantomData  (0 B ZST)
/// +8  : end
/// ```
///
/// `#[repr(C)]` for the documented compact layout; `PhantomData` is ZST so
/// total size is exactly 8 B.
#[repr(C)]
pub(crate) struct RemoveCommand<C: Component> {
    pub(crate) entity: Entity,
    pub(crate) _marker: PhantomData<C>,
}

impl<C: Component> RemoveCommand<C> {
    /// Crate-internal constructor used by
    /// [`EntityCommands::remove`](crate::ecs::core::system::params::entity_commands::EntityCommands::remove).
    #[inline]
    pub(crate) const fn new(entity: Entity) -> Self {
        Self { entity, _marker: PhantomData }
    }
}

// SAFETY (mirrors B3): `C: Component` does not currently require Send/Sync,
//   but `Component: 'static` (registry contract) is sufficient for this
//   queue payload because `PhantomData<C>` is unconditionally Send + Sync
//   when `C: 'static`. The explicit impls document the intent.
unsafe impl<C: Component> Send for RemoveCommand<C> {}
unsafe impl<C: Component> Sync for RemoveCommand<C> {}

impl<C: Component> Command for RemoveCommand<C> {
    fn apply(self, world: &mut EcsMaster) {
        let entity = self.entity;

        // Resolve source archetype id via the fast inland.
        let inland = match world.entity_master.entities_inland.get(entity.id().0) {
            Some(slot) => *slot,
            None => {
                debug_assert!(false, "RemoveCommand::apply: entity {:?} never registered", entity);
                return; // EC8 silent no-op in release
            }
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            debug_assert!(false, "RemoveCommand::apply: stale entity {:?}", entity);
            return;
        }

        // Dense plan D2 / decision 3: a dense `C` is NOT in any archetype
        // signature, so `without_component_archetype_id::<C>` would (wrongly)
        // return `None` (silent no-op) — branch FIRST. Dense remove tombstones the
        // membership in `C`'s `DenseStore` + fires on_replace/on_remove, with NO
        // archetype migration (the table component set is untouched). A no-op if
        // the entity is not a member (matching the table absent-component no-op).
        // For a table `C` this branch folds out (the 0%-gate: the const-true
        // `matches!` is a cold registration-table read, then the existing path).
        if matches!(component_registry::storage_kind(C::component_id().0), StorageKind::Dense) {
            world.dense_remove_and_fire(entity, C::component_id());
            return;
        }

        // SAFETY (U1, U2, U11, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survives sibling
        //   structural writes (e.g. a later spawn's `current_index += 1` through
        //   a same-cell-derived pointer) under TB/SB because the whole slab
        //   element is `UnsafeCell`-wrapped. The pointer is non-null and matches
        //   the entity generation (checked above), so the slot is live.
        let source_archetype_id = unsafe { (*inland.archetype_ptr()).id() };

        // W1: absent C ⇒ silent no-op (NO debug_assert).
        let Some(target_archetype_id) =
            without_component_archetype_id::<C>(world, source_archetype_id)
        else {
            return;
        };

        // `target_archetype_id == source_archetype_id` is unreachable
        // here because `without_component_archetype_id` only returns
        // `Some` when C is hosted by source — `kept` is strictly smaller
        // than `source.component_ids()`, so `get_or_create_archetype`
        // yields a distinct id.
        debug_assert_ne!(
            target_archetype_id, source_archetype_id,
            "without_component_archetype_id returned source_archetype_id (regression)"
        );

        migrate_entity_remove::<C>(world, entity, source_archetype_id, target_archetype_id);
    }
}
