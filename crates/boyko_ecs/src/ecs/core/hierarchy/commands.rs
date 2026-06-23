//! Hierarchy-specific deferred commands + the cascade-suppress thread-local
//! (Phase 19, CORE — refactored onto the generic Relations machinery).
//!
//! # Relations Option A
//!
//! The Phase-19 `child_of_on_insert` / `child_of_on_replace` /
//! `children_on_replace` hook bodies and the `LinkChildCommand` /
//! `UnlinkChildCommand` deferred commands are DELETED — `ChildOf` / `Children`
//! are now specializations of the generic
//! [`Relationship`](crate::ecs::core::relationship::Relationship) /
//! [`RelationshipTarget`](crate::ecs::core::relationship::RelationshipTarget)
//! machinery. The link/unlink/cascade logic lives ONCE in
//! [`generic_hooks`](crate::ecs::core::relationship::generic_hooks), wired into
//! the hierarchy's `register_hooks` (see [`super`]).
//!
//! What remains here is HIERARCHY-SPECIFIC ergonomics that the generic API does
//! not (yet) cover:
//!
//! * [`ClearChildrenCommand`] — backs `clear_children` (remove `ChildOf` from
//!   every current child of a parent).
//! * [`DespawnWithoutChildrenCommand`] — backs
//!   `EntityCommands::despawn_without_children` (the cascade opt-out).
//! * The [`CASCADE_SUPPRESS`] thread-local + [`CascadeSuppressGuard`] — the
//!   kernel-level, relation-agnostic cascade opt-out the generic cascade body
//!   reads via [`cascade_suppressed`].

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::remove_command::RemoveCommand;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::ChildOf;

// ===========================================================================
// Hierarchy-specific deferred commands (Phase 19 W2 / W5)
// ===========================================================================

/// Deferred "remove `ChildOf` from every current child of `parent`" command
/// (Phase 19 W2). Backs `clear_children` (not `remove_children`, which removes
/// `ChildOf` from its listed children directly). Each removal fires
/// [`ChildOf`]'s `on_replace`, which enqueues the generic unlink.
#[repr(C)]
pub(crate) struct ClearChildrenCommand {
    pub(crate) parent: Entity,
}

/// Deferred "despawn `entity` WITHOUT cascading to its children" command
/// (Phase 19 W5). Backs `EntityCommands::despawn_without_children` — routes to
/// [`EcsMaster::despawn_without_children`] at the apply window so the cascade
/// suppress is active for exactly the one removal.
#[repr(C)]
pub(crate) struct DespawnWithoutChildrenCommand {
    pub(crate) entity: Entity,
}

// SAFETY (mirrors `InsertCommand` / B3): the payloads are plain `Entity` PODs
//   (one `usize`-ish word, no borrowed references), so moving them across
//   threads is sound. The explicit impls document the intent for the
//   `Command: Send + 'static` queue bound.
unsafe impl Send for ClearChildrenCommand {}
unsafe impl Sync for ClearChildrenCommand {}
unsafe impl Send for DespawnWithoutChildrenCommand {}
unsafe impl Sync for DespawnWithoutChildrenCommand {}

impl Command for ClearChildrenCommand {
    fn apply(self, world: &mut EcsMaster) {
        // #17883-safe: re-read the CURRENT children per turn, copy one child out,
        // drop the borrow, THEN enqueue. Each child's `ChildOf` removal is
        // DEFERRED (pushed to the queue, applied on a later drain turn), so the
        // `Children` collection does NOT shrink during this loop — iterating by
        // index `0..len` visits each child exactly once. Routing through
        // `RemoveCommand<ChildOf>` fires `ChildOf::on_replace` → the audited
        // generic unlink path, so the collection empties consistently.
        let mut i = 0usize;
        loop {
            let next = {
                let Some(children) = world.get_component::<Children>(self.parent) else {
                    return;
                };
                if i >= children.len() {
                    return;
                }
                children.as_slice()[i]
                // <-- `&Children` drops here.
            };
            world.enqueue_child_of_removal(next);
            i += 1;
        }
    }
}

impl Command for DespawnWithoutChildrenCommand {
    #[inline]
    fn apply(self, world: &mut EcsMaster) {
        // Routes to the direct API, which scopes the cascade-suppress guard to
        // exactly this removal's hook fire. Its internal drain no-ops at depth
        // >= 1 (this runs inside the outermost drain), so the outermost owner
        // still applies any commands the despawn's other hooks enqueue.
        world.despawn_without_children(self.entity);
    }
}

impl EcsMaster {
    /// Enqueues a single deferred `RemoveCommand<ChildOf>` for `child` (used by
    /// [`ClearChildrenCommand::apply`]). Enqueuing rather than removing inline
    /// keeps the unlink on the audited `migrate_entity_remove` path and fires
    /// `ChildOf::on_replace` exactly once per child.
    #[inline]
    fn enqueue_child_of_removal(&mut self, child: Entity) {
        self.deferred_hook_queue.push(RemoveCommand::<ChildOf>::new(child));
    }
}

// ===========================================================================
// Cascade-suppress thread-local (W4) — kernel-level, relation-agnostic
// ===========================================================================

use std::cell::Cell;

use crate::ecs::core::hierarchy::Children;

thread_local! {
    /// When set, the generic `LINKED_DESPAWN` cascade body returns immediately —
    /// the `despawn_without_children` opt-out. Thread-local for the same reason as
    /// `HOOK_DRAIN_DEPTH` (`hooks/scope.rs`): all hook firing runs on the
    /// single-threaded apply window, and a per-thread cell cannot be frozen by
    /// any `&mut EcsMaster` reborrow (the F2 invariant).
    static CASCADE_SUPPRESS: Cell<bool> = const { Cell::new(false) };
}

/// Reads the cascade-suppress flag for the current thread. Read by the generic
/// cascade body
/// ([`relationship_target_on_replace`](crate::ecs::core::relationship::generic_hooks::relationship_target_on_replace)).
#[inline]
pub(crate) fn cascade_suppressed() -> bool {
    CASCADE_SUPPRESS.with(|s| s.get())
}

/// RAII guard that sets the cascade-suppress flag for its scope and clears it on
/// every exit path (`Ok` / panic). Mirrors `DeferredScopeGuard`
/// (`hooks/scope.rs`) — it touches only the thread-local, never any field of
/// `EcsMaster`, so a bracketed `&mut self` body cannot freeze it.
pub(crate) struct CascadeSuppressGuard {
    /// Restores the previous flag value on drop (supports nesting, though in
    /// practice the depth is one).
    prev: bool,
}

impl CascadeSuppressGuard {
    /// Enters a cascade-suppressed scope.
    #[inline]
    pub(crate) fn enter() -> Self {
        let prev = CASCADE_SUPPRESS.with(|s| {
            let p = s.get();
            s.set(true);
            p
        });
        Self { prev }
    }
}

impl Drop for CascadeSuppressGuard {
    #[inline]
    fn drop(&mut self) {
        CASCADE_SUPPRESS.with(|s| s.set(self.prev));
    }
}
