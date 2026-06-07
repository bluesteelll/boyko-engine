//! `DeferredEcsMaster<'w>` — the restricted read-only view handed to lifecycle
//! hooks (Phase 14a, plan §2.1 / Q-A2 / C2).
//!
//! A hook fires synchronously at a structural-op site while the outermost
//! apply holds `&mut EcsMaster`. The view is minted from that same `&mut` and
//! statically WITHHOLDS:
//!
//! * every structural-change method (`create_entity` / `delete_entity` /
//!   `spawn_*`) — structural change is *deferred* via the `commands` handle; and
//! * every `&mut`-into-archetype-storage method (`get_component_mut` /
//!   `set_component_raw`) — dropped per Q-A2.
//!
//! With no `&mut`-into-storage method, a hook *cannot construct* an aliasing
//! `&mut` into a component pool buffer (C2 closed: the non-aliasing obligation
//! is a missing method, not a documented contract). Mutable component access
//! is deferred to 14b. There is intentionally **no** `Deref<Target =
//! EcsMaster>` — that would leak the full structural + mutable API.
//!
//! Wave 4 mints this type from the `trigger_on_*` dispatch fns. The view's
//! read methods (`get_component` / `resource` / `current_tick`) and the
//! `commands` handle are part of the public `HookFn` surface — exercised by
//! user hooks, not by the crate internals, so the deferred-command enqueue
//! methods carry an item-level `#[allow(dead_code)]` rather than the
//! module-level blanket that Waves 1-3 used.

use core::marker::PhantomData;
use core::sync::atomic::Ordering;
use std::ptr::NonNull;

use crate::ecs::core::bundle::bundle::Bundle;
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::despawn_command::DespawnCommand;
use crate::ecs::core::commands::insert_command::InsertCommand;
use crate::ecs::core::commands::remove_command::RemoveCommand;
use crate::ecs::core::commands::spawn_at_command::SpawnAtCommand;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::resources::resource::Resource;
use crate::ecs::identifiers::primitives::EntityId;

/// Restricted READ-ONLY view of [`EcsMaster`] handed to lifecycle hooks (14a).
///
/// Exposes component READS, resource access, a deferred-command handle, and
/// the current tick — and statically withholds every structural-change and
/// `&mut`-into-storage method (plan §2.1 / SAFETY-1).
///
/// # Layout
///
/// `#[repr(transparent)]` over a raw [`NonNull<EcsMaster>`] (not `&mut`), so
/// the outermost apply's `*mut` reborrows are not invalidated under Tree
/// Borrows. The `PhantomData<&'w mut EcsMaster>` ties the apparent borrow to
/// `'w`.
#[repr(transparent)]
pub struct DeferredEcsMaster<'w> {
    /// Raw `NonNull`, NOT `&mut`. Minted from the same `&mut EcsMaster` the
    /// apply holds, AFTER every `world`-derived `&mut Archetype` is dead
    /// (per-site liveness, plan §3).
    world: NonNull<EcsMaster>,
    _marker: PhantomData<&'w mut EcsMaster>,
}

impl<'w> DeferredEcsMaster<'w> {
    /// Mints the view from a raw world pointer.
    ///
    /// # Safety
    ///
    /// * `world` points at a live [`EcsMaster`] the caller holds exclusively
    ///   (minted via `NonNull::from(&mut *world)` at a `trigger_on_*` site).
    /// * No `world`-derived `&mut Archetype` / `&mut ComponentPool` is live at
    ///   the mint point (plan §3 per-site liveness; SAFETY-1).
    /// * Invoked only inside the single-threaded apply window (SAFETY-4).
    #[inline]
    pub(crate) unsafe fn from_world(world: NonNull<EcsMaster>) -> Self {
        Self { world, _marker: PhantomData }
    }

    /// Reads a component of a (possibly different) entity. Read-only: resolves
    /// a fresh `*const` via [`EcsMaster::get_component`] per call, so no cached
    /// aliasing pointer survives.
    #[inline]
    pub fn get_component<T: Component>(&self, e: Entity) -> Option<&T> {
        // SAFETY: `from_world`'s contract guarantees `self.world` is live and
        //   exclusively borrowed for `'w`, and no `&mut`-into-storage is live.
        //   The returned `&T` borrows `self` (lifetime tied below), so it
        //   cannot outlive the view.
        let world: &EcsMaster = unsafe { self.world.as_ref() };
        world.get_component::<T>(e)
    }

    /// Reads a resource. Resources live OUTSIDE archetype storage, so this
    /// never aliases the apply's component writes.
    #[inline]
    pub fn resource<R: Resource>(&self) -> Option<&R> {
        // SAFETY: same exclusive-borrow contract as `get_component`.
        let world: &EcsMaster = unsafe { self.world.as_ref() };
        world.try_resource::<R>()
    }

    /// Mutates a resource. Resources live OUTSIDE archetype storage — never
    /// aliases the apply's component writes (the canonical `on_remove`
    /// "decrement a counter" pattern).
    #[inline]
    pub fn resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        // SAFETY: `&mut self` ⇒ exclusive view; the world is exclusively
        //   borrowed per `from_world`. Resources are disjoint from archetype
        //   storage, so the `&mut R` cannot alias any component pool buffer.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.try_resource_mut::<R>()
    }

    /// Reads the current change-detection tick.
    #[inline]
    pub fn current_tick(&self) -> Tick {
        // SAFETY: shared read of a live, exclusively-borrowed world.
        let world: &EcsMaster = unsafe { self.world.as_ref() };
        world.current_tick()
    }

    /// Returns `true` iff `entity` is currently live (Phase 19 §3 — the
    /// `ChildOf::on_insert` dangling-parent guard).
    ///
    /// A read-only existence check delegating to [`EcsMaster::has_entity`]; it
    /// takes no `&mut`-into-storage, so it adds no Tree-Borrows surface. Named
    /// `has_parent` because its sole caller probes a prospective parent, but it
    /// is a generic liveness check on any entity.
    #[inline]
    pub fn has_parent(&self, entity: Entity) -> bool {
        // SAFETY: same exclusive-borrow contract as `get_component`; this is a
        //   shared read of a live, exclusively-borrowed world.
        let world: &EcsMaster = unsafe { self.world.as_ref() };
        world.has_entity(entity)
    }

    /// Returns a [`DeferredCommands`] handle that enqueues structural commands
    /// into the world-resident deferred queue (plan §2.1 / Q-A1). Commands are
    /// drained at the OUTERMOST apply boundary — never inline.
    #[inline]
    pub fn commands(&mut self) -> DeferredCommands<'_> {
        DeferredCommands { world: self.world, _marker: PhantomData }
    }
}

/// Deferred-command handle minted by [`DeferredEcsMaster::commands`].
///
/// Pushes structural commands into `EcsMaster::deferred_hook_queue`
/// (Phase 14a, plan §2.1). The handle is shaped after `Commands` but routes
/// into the world-resident queue rather than a per-system one, so a hook
/// firing mid-apply reaches a queue that is NOT borrow-frozen on the stack
/// above.
///
/// The enqueued commands are applied only when the outermost
/// `EcsMaster::drain_deferred_hook_queue` runs (depth 0). Nothing in
/// Waves 1-3 constructs this handle; Wave 4 wires the dispatch.
pub struct DeferredCommands<'a> {
    /// Raw pointer to the world owning `deferred_hook_queue`. Mirrors the
    /// `DeferredEcsMaster` discipline: a raw `NonNull`, not `&mut`, so the
    /// outermost apply's reborrows stay valid.
    world: NonNull<EcsMaster>,
    /// Ties `'a` (the borrow scope of the `&mut DeferredEcsMaster` that minted
    /// this handle) to the handle, so it cannot outlive the view.
    _marker: PhantomData<&'a mut EcsMaster>,
}

impl<'a> DeferredCommands<'a> {
    /// Enqueues an arbitrary user-defined [`Command`] for deferred apply.
    #[inline]
    pub fn add<C: Command>(&mut self, cmd: C) {
        // SAFETY: the world pointer is live and exclusively borrowed for the
        //   apply window (DeferredEcsMaster::from_world contract). Pushing into
        //   the queue mutates only `deferred_hook_queue`'s heap buffer; no
        //   `&mut Archetype` is live, and the drain runs strictly later.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.deferred_hook_queue.push(cmd);
    }

    /// Reserves a fresh [`Entity`] via the world's atomic counter and enqueues
    /// a `SpawnAtCommand<B>`. The entity is not yet live; it materialises at
    /// the next outermost drain. Mirrors `Commands::spawn` minus the chaining
    /// return (14a exposes a single-shot handle).
    #[inline]
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
        // SAFETY: exclusive world borrow per the apply-window contract.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        // Reserve via the same atomic counter `Commands::spawn` uses
        // (`fetch_add(1, Relaxed)` — uniqueness only; EM4).
        let id = world.entity_master.next_id_atomic().fetch_add(1, Ordering::Relaxed);
        debug_assert!(id < usize::MAX / 2, "EntityId counter near exhaustion");
        let entity = Entity::new(EntityId(id), 0);
        world.deferred_hook_queue.push(SpawnAtCommand { entity, bundle });
        entity
    }

    /// Returns a per-entity [`DeferredEntityCommands`] handle for chaining
    /// `insert` / `remove` / `despawn` over an existing entity.
    #[inline]
    pub fn entity(&mut self, entity: Entity) -> DeferredEntityCommands<'_> {
        DeferredEntityCommands { world: self.world, entity, _marker: PhantomData }
    }

    /// Convenience wrapper for `entity(e).despawn()`.
    #[inline]
    pub fn despawn(&mut self, entity: Entity) {
        // SAFETY: exclusive world borrow per the apply-window contract.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.deferred_hook_queue.push(DespawnCommand { entity });
    }
}

/// Per-entity deferred-command handle minted by [`DeferredCommands::entity`].
///
/// Mirrors the `EntityCommands` enqueue surface (`insert` / `remove` /
/// `despawn`) but pushes into the world-resident deferred queue (Phase 14a).
/// Methods take `&mut self` and return `&mut Self` for chaining.
pub struct DeferredEntityCommands<'a> {
    world: NonNull<EcsMaster>,
    entity: Entity,
    /// Ties `'a` (the borrow of the parent `DeferredCommands`) to the handle.
    _marker: PhantomData<&'a mut EcsMaster>,
}

impl<'a> DeferredEntityCommands<'a> {
    /// Returns the entity this handle targets.
    #[inline]
    pub fn id(&self) -> Entity {
        self.entity
    }

    /// Enqueues an `InsertCommand<B>` for this entity. Chainable.
    #[inline]
    pub fn insert<B: Bundle>(&mut self, bundle: B) -> &mut Self {
        // SAFETY: exclusive world borrow per the apply-window contract.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.deferred_hook_queue.push(InsertCommand { entity: self.entity, bundle });
        self
    }

    /// Enqueues a `RemoveCommand<C>` for this entity. Chainable.
    #[inline]
    pub fn remove<C: Component>(&mut self) -> &mut Self {
        // SAFETY: exclusive world borrow per the apply-window contract.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.deferred_hook_queue.push(RemoveCommand::<C>::new(self.entity));
        self
    }

    /// Enqueues a [`DespawnCommand`] for this entity.
    #[inline]
    pub fn despawn(&mut self) {
        // SAFETY: exclusive world borrow per the apply-window contract.
        let world: &mut EcsMaster = unsafe { self.world.as_mut() };
        world.deferred_hook_queue.push(DespawnCommand { entity: self.entity });
    }
}
