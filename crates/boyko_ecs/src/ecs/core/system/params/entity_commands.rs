//! `EntityCommands<'a, 's>` — chainable per-entity deferred-command handle.
//!
//! Phase 11 (plan §5.1, §5.3). Returned from
//! [`Commands::spawn`](super::commands::Commands::spawn) and
//! [`Commands::entity`](super::commands::Commands::entity); exposes
//! `.insert(...).insert(...).remove::<X>().despawn().id()` chaining over a
//! single [`Entity`].
//!
//! # Lifetimes (C1 — two-lifetime shape)
//!
//! * `'a` — borrow scope of `&'a mut Commands<'s>`. Shorter or equal to
//!   `'s` (implicit via the `&'a mut` field). A reborrow via
//!   [`reborrow`](EntityCommands::reborrow) produces an `EntityCommands<'_, 's>`
//!   with a shorter `'a` and the same `'s`.
//! * `'s` — the system's state scope (the lifetime of the underlying
//!   [`CommandQueue`](crate::ecs::core::commands::command_queue::CommandQueue)).
//!   Preserved across reborrow so subsequent enqueues stay in the same
//!   per-system queue.
//!
//! Bevy's modern `EntityCommands<'a>` collapses `'s` into a single
//! lifetime because the world is borrowed for the entire schedule pass.
//! boyko-engine's two-lifetime shape (matching Bevy's pre-cleanup form) is
//! mandated by C1 — single-lifetime `Commands<'s>` would make
//! `EntityCommands<'a>` invariant in `'a` and reborrow would fail at the
//! borrow checker (see plan §5.1).
//!
//! # Send / Sync (EC1)
//!
//! `EntityCommands` is `!Send + !Sync` — `&'a mut Commands<'s>` is `!Sync`
//! by CQ-SEND2 and the borrow itself is local-thread.

#![allow(dead_code)]

use crate::ecs::core::bundle::Bundle;
use crate::ecs::core::commands::despawn_command::DespawnCommand;
use crate::ecs::core::commands::insert_command::InsertCommand;
use crate::ecs::core::commands::remove_command::RemoveCommand;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::commands::ClearChildrenCommand;
use crate::ecs::core::hierarchy::ChildOf;
use crate::ecs::core::system::params::commands::Commands;

/// Chainable handle for issuing per-entity deferred commands (Phase 11
/// §5.1 / EC1..EC15).
///
/// Construct via [`Commands::spawn`] (fresh entity, pre-allocated via the
/// atomic counter) or [`Commands::entity`] (existing or arbitrary entity).
/// Methods consume `&mut self` and return `&mut Self` for chaining; the
/// terminal [`id`](Self::id) accessor returns the captured [`Entity`].
///
/// # Layout (plan §11.1, EC15 — adjusted for boyko-engine's `usize` IDs)
///
/// ```text
/// +0  : entity.id: EntityId         (8 B; usize on 64-bit per
///                                     ecs::identifiers::primitives)
/// +8  : entity.generation: u32      (4 B)
/// +12 : padding                     (4 B — Entity carries it; not added
///                                     by this struct)
/// +16 : commands: &'a mut Commands  (8 B: pointer)
/// +24 : end
/// ```
///
/// 24 B on 64-bit. The plan §11.1's 16 B figure assumed a 4-byte
/// `EntityId`; boyko-engine ships `EntityId(pub usize)` for cross-id
/// compatibility (see `ecs::identifiers::primitives`). The plan's
/// "one cache line" cache-budget claim still holds (24 B « 64 B line).
///
/// `#[repr(C)]` pins the field order so future macro-driven layout
/// inspection stays stable.
///
/// [`Commands::spawn`]: super::commands::Commands::spawn
/// [`Commands::entity`]: super::commands::Commands::entity
#[repr(C)]
pub struct EntityCommands<'a, 's> {
    /// The entity targeted by this handle. Captured at construction —
    /// either pre-allocated by `Commands::spawn` via the atomic counter
    /// or supplied verbatim by `Commands::entity`. EC2: always real.
    pub(crate) entity: Entity,

    /// Borrow of the parent `Commands<'s>` for the chain's duration.
    /// EC1: `&mut Commands<'s>` is `!Sync` (CQ-SEND2) so this struct is
    /// `!Send + !Sync`. The borrow itself carries the `'s` lifetime —
    /// no separate `PhantomData<&'s _>` is needed.
    pub(crate) commands: &'a mut Commands<'s>,
}

// EC15: compile-time guard on the layout — locked at 24 B for the
// engine's `usize`-backed `EntityId` (see the layout doc above for the
// plan-vs-implementation reconciliation). A future field addition would
// trip this assert before silently shipping a perf regression.
// The `usize`-backed `Entity` plus a `&mut Commands` reference make the 24-byte
// size encode the 64-bit ABI; gated to 64-bit (the engine's supported platform)
// — see CLAUDE.md target platform.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<EntityCommands<'static, 'static>>() == 24);

impl<'a, 's> EntityCommands<'a, 's> {
    /// Constructs a fresh `EntityCommands` handle (crate-internal —
    /// `Commands::spawn` and `Commands::entity` are the blessed public
    /// constructors).
    #[inline]
    pub(crate) fn new(entity: Entity, commands: &'a mut Commands<'s>) -> Self {
        Self { entity, commands }
    }

    /// Returns the entity targeted by this handle. **EC2 — infallible.**
    ///
    /// For freshly-spawned entities (from `Commands::spawn`) the ID was
    /// minted by the atomic counter and is guaranteed real *at counter
    /// level*; the entity becomes query-visible only after
    /// `CommandQueue::apply` materialises the `SpawnAtCommand` (EC13).
    #[inline]
    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Enqueues an [`InsertCommand<B>`] for this entity. Chainable
    /// (EC3 — `&mut self → &mut Self`).
    ///
    /// On apply, the entity's archetype either grows by `B`'s components
    /// (migrate) or the components are replaced in place if `B`'s id set
    /// is a subset of the source archetype (EC9 + plan §7.4 fast path).
    ///
    /// # Cost
    ///
    /// `~18 ns` per call — one `CommandQueue::push` (plan §10.1).
    #[inline]
    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        self.commands.queue.push(InsertCommand { entity: self.entity, bundle });
        self
    }

    /// Enqueues a [`RemoveCommand<C>`] removing component `C` from this
    /// entity. Chainable.
    ///
    /// If the entity's archetype does not host `C` at apply time, the
    /// command silently no-ops (W1 — Bevy Issue #10166). Bundle-typed
    /// remove is deferred to Phase 12 (EC10).
    #[inline]
    pub fn remove<C: Component>(&mut self) -> &mut Self {
        self.commands.queue.push(RemoveCommand::<C>::new(self.entity));
        self
    }

    /// Enqueues a [`DespawnCommand`] for this entity. Chainable
    /// (Q3 — Bevy PR #15523 revert lesson).
    ///
    /// Despawn is **not** terminal by signature; subsequent chain calls
    /// still compile (and enqueue post-despawn commands that will
    /// debug_assert + no-op on a freed entity). The borrow checker will
    /// not protect the user from logical errors of this shape.
    #[inline]
    pub fn despawn(&mut self) -> &mut Self {
        self.commands.queue.push(DespawnCommand { entity: self.entity });
        self
    }

    /// Phase 12 placeholder — currently identical to
    /// [`insert`](Self::insert). The name is reserved for the future
    /// "only-if-new" semantic plus the output-slot success indicator
    /// (EC12 + OQ6).
    #[inline]
    pub fn try_insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator — do NOT alias
        // the non-`try_*` form once the slot machinery lands.
        self.insert(bundle)
    }

    /// Phase 12 placeholder — currently identical to
    /// [`remove`](Self::remove). See [`try_insert`](Self::try_insert) for
    /// the deferred-semantic rationale.
    #[inline]
    pub fn try_remove<C: Component>(&mut self) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator.
        self.remove::<C>()
    }

    /// Phase 12 placeholder — currently identical to
    /// [`despawn`](Self::despawn). See [`try_insert`](Self::try_insert).
    #[inline]
    pub fn try_despawn(&mut self) -> &mut Self {
        // TODO Phase 12: wire output-slot success indicator.
        self.despawn()
    }

    /// Reborrows the handle with a shorter `'a` lifetime, preserving
    /// `'s` (EC4 / plan §5.1).
    ///
    /// Used to pass a shorter-lifetime handle into a helper while still
    /// allowing the original handle to remain usable after the helper
    /// returns:
    ///
    /// ```ignore
    /// fn complex_helper(mut ec: EntityCommands<'_, '_>) {
    ///     helper_subroutine(ec.reborrow());  // shorter lifetime
    ///     ec.insert(MoreComponents);          // original still usable
    /// }
    /// ```
    #[inline]
    pub fn reborrow(&mut self) -> EntityCommands<'_, 's> {
        EntityCommands { entity: self.entity, commands: &mut *self.commands }
    }

    // ── Hierarchy ergonomics (Phase 19 W5) ──────────────────────────────────
    //
    // The relationship is driven ENTIRELY by `ChildOf` insertion / removal —
    // these are thin wrappers that never write `Children` directly. `Children`
    // is maintained by `ChildOf`'s hooks (see `crate::ecs::core::hierarchy`).

    /// Adds `child` as a child of this entity by inserting [`ChildOf`] on the
    /// child (Phase 19). Chainable.
    ///
    /// Equivalent to `commands.entity(child).set_parent(this)`. The link
    /// materialises at the next deferred-command drain.
    #[inline]
    pub fn add_child(&mut self, child: Entity) -> &mut Self {
        let parent = self.entity;
        self.commands
            .queue
            .push(InsertCommand { entity: child, bundle: ChildOf(parent) });
        self
    }

    /// Adds every entity in `children` as a child of this entity (Phase 19).
    /// Chainable. One [`ChildOf`] insert per child.
    #[inline]
    pub fn add_children(&mut self, children: &[Entity]) -> &mut Self {
        let parent = self.entity;
        for &child in children {
            self.commands
                .queue
                .push(InsertCommand { entity: child, bundle: ChildOf(parent) });
        }
        self
    }

    /// Sets this entity's parent to `parent` by inserting [`ChildOf`] (Phase
    /// 19). Chainable.
    ///
    /// If this entity already had a parent, the overwrite reparents it
    /// atomically (the old parent's `on_replace` unlink is applied before the
    /// new parent's `on_insert` link — FIFO drain order).
    #[inline]
    pub fn set_parent(&mut self, parent: Entity) -> &mut Self {
        self.insert(ChildOf(parent))
    }

    /// Removes this entity's parent link by removing [`ChildOf`] (Phase 19).
    /// Chainable. A no-op if this entity has no parent.
    #[inline]
    pub fn remove_parent(&mut self) -> &mut Self {
        self.remove::<ChildOf>()
    }

    /// Removes every entity in `children` from this entity's children by
    /// removing [`ChildOf`] from each (Phase 19). Chainable.
    ///
    /// Each child whose parent is actually this entity is unlinked; a child
    /// whose `ChildOf` points elsewhere is unlinked from THAT parent (the
    /// removal is unconditional on the child). The children are NOT despawned.
    #[inline]
    pub fn remove_children(&mut self, children: &[Entity]) -> &mut Self {
        for &child in children {
            self.commands.queue.push(RemoveCommand::<ChildOf>::new(child));
        }
        self
    }

    /// Removes [`ChildOf`] from ALL current children of this entity, clearing
    /// its [`Children`](crate::ecs::core::hierarchy::Children) without
    /// despawning them (Phase 19). Chainable.
    ///
    /// Re-reads the current children per turn at apply time (#17883-safe) and
    /// routes a deferred `ChildOf` removal per child (the deferred removals do
    /// not shrink the collection mid-walk, so each child is visited once).
    #[inline]
    pub fn clear_children(&mut self) -> &mut Self {
        let parent = self.entity;
        self.commands.queue.push(ClearChildrenCommand { parent });
        self
    }

    /// Despawns this entity WITHOUT cascading to its children (Phase 19).
    /// Chainable.
    ///
    /// The children survive with a now-dangling [`ChildOf`] (documented
    /// footgun); see
    /// [`EcsMaster::despawn_without_children`](crate::ecs::core::ecs_master::ecs_master::EcsMaster::despawn_without_children).
    /// Note `iter_descendants` is a future addition — descent today is
    /// `Query<&Children>` + manual recursion.
    #[inline]
    pub fn despawn_without_children(&mut self) -> &mut Self {
        self.commands
            .queue
            .push(crate::ecs::core::hierarchy::commands::DespawnWithoutChildrenCommand {
                entity: self.entity,
            });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EC15 / O1: `EntityCommands<'_, '_>` is 24 B on the engine's
    /// `usize`-backed `EntityId` layout (plan-vs-impl reconciliation).
    #[test]
    fn entity_commands_size_is_24_bytes() {
        assert_eq!(core::mem::size_of::<EntityCommands<'static, 'static>>(), 24);
    }
}
