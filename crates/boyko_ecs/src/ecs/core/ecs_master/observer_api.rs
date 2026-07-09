//! Hooks, observers, and trigger dispatch surface on [`EcsMaster`] (mechanical
//! split).
//!
//! Component lifecycle hooks / observers, entity-targeted observers, custom
//! triggers, relation-edge observers, and the trigger-walk machinery. Extracted
//! verbatim from `ecs_master.rs`.
use std::ptr::NonNull;

use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self};
use crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags;
use crate::ecs::core::component::hooks::builder::ComponentHooksBuilder;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::component::observers::entity_store::fire_entity_triggers;
use crate::ecs::core::component::observers::propagate::{PropagateGuard, get_propagate, propagate};
use crate::ecs::core::component::observers::traversal::{PropagationMode, Traversal};
use crate::ecs::core::component::observers::trigger::{
    Trigger, TriggerContext, TriggerFn, TriggerId, fire_global_triggers,
    static_trigger_id,
};
use crate::ecs::core::iters::query::relation::traverse_iter::VisitedSet;
use crate::ecs::core::relationship::{
    OnLink, OnUnlink, Relationship, RelationshipSourceCollection, RelationshipTarget,
};
use crate::ecs::core::component::observers::{ObserverFn, ObserverId, ObserverKind};
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;
use crate::ecs::identifiers::primitives::{
    ComponentId, EntityId,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    /// Registers lifecycle hooks for component type `C` at runtime, returning a
    /// chainable [`ComponentHooksBuilder`] (Phase 14a, plan §6.3 / REG).
    ///
    /// ```ignore
    /// world.register_component_hooks::<Health>()
    ///      .on_add(my_on_add)
    ///      .on_remove(my_on_remove);
    /// ```
    ///
    /// The builder commits the accumulated hooks when it is dropped (or
    /// [`finish`](ComponentHooksBuilder::finish)ed). It is the runtime
    /// counterpart of the `#[component(...)]` derive attribute and covers
    /// hand-written `impl Component` / foreign types that the derive cannot
    /// reach.
    ///
    /// # Derive XOR runtime (mutually exclusive)
    ///
    /// A component declares hooks via EITHER `#[component(...)]` OR this runtime
    /// builder — never both. Each `HOOKS` slot is written exactly once. Calling
    /// this method for a type that carries `#[component(...)]` (i.e.
    /// `C::HAS_HOOKS == true`) panics immediately: the derive already installed
    /// the slot, and the two mechanisms must not be mixed.
    ///
    /// # Register-before-use (staleness rule, plan §6.4 / Q-A5; Phase 21 H1)
    ///
    /// Hooks for `C` MUST be registered before `C` first appears in any
    /// archetype **of any world in this process**. An archetype's
    /// [`ArchetypeFlags`] are OR-computed once at construction from the cold
    /// `HOOKS` table; hooks installed *after* an archetype containing `C`
    /// already exists would leave that archetype's flag bit unset and the hook
    /// silently skipped. To make that bug impossible, this method checks the
    /// process-global per-`ComponentId` "ever placed in any archetype" bitmask
    /// (set at every archetype-creation funnel) and **panics** (in release,
    /// not just debug) if the bit is set. The pre-Phase-21 per-world archetype
    /// scan was world-blind: a SECOND world with `C` live would get
    /// silently-skipped hooks because its pre-install archetypes' flags lacked
    /// the bit — the global gate closes that hole. The derive path is
    /// staleness-immune by construction (hooks install inside
    /// `component_id()`, which always precedes the first archetype containing
    /// the component).
    ///
    /// # Multi-world scope (Phase 21)
    ///
    /// Hooks are **process-global per type** — registered once, they fire in
    /// ALL worlds (the `HOOKS` table is `static`). Observers, by contrast, are
    /// **per-world** (`ObserverRegistry` lives on each world's
    /// `ArchetypeMaster`). The asymmetry is by design: hooks are part of a
    /// component type's definition; observers are runtime-mutable per-world
    /// reactions.
    ///
    /// # Panics
    ///
    /// - If `C` declares `#[component(...)]` derive hooks (`C::HAS_HOOKS ==
    ///   true`) — derive and the runtime builder are mutually exclusive.
    /// - If `C` was ever placed in a live archetype of ANY world in this
    ///   process — register hooks before the component is first used.
    #[cold]
    pub fn register_component_hooks<C: Component>(&mut self) -> ComponentHooksBuilder<'_> {
        // Force `C::component_id()`: mints the id and, for a derive-hooked type
        // (`C::HAS_HOOKS == true`), installs those hooks into the slot. A plain
        // `#[derive(Component)]` installs nothing, leaving the slot free for the
        // runtime builder to commit.
        let component_id = C::component_id();

        // Eager derive-XOR-runtime collision check (Wave-5 soundness fix /
        // Change 3): a type carrying `#[component(...)]` already owns its `HOOKS`
        // slot, so the runtime builder must not also write it. Reject at the
        // registration call site — a clearer, earlier error than the builder's
        // `Drop` commit panic (which remains as defense in depth for a
        // hand-`impl Component` with an inconsistent `HAS_HOOKS`).
        if C::HAS_HOOKS {
            register_component_hooks_derive_conflict_panic::<C>();
        }

        // Release-level staleness gate (Q-A5 / W3; Phase 21 H1): a stale
        // `ArchetypeFlags` bit would silently skip the hook, which is too
        // severe a correctness surprise for a feature whose entire value is
        // "the callback fires". The gate is the PROCESS-GLOBAL "ever placed in
        // any archetype" bitmask — matching the process-global scope of the
        // `HOOKS` table itself — because the old per-world archetype scan was
        // blind to other worlds already holding `C` (audit H1). The global
        // subsumes the per-world scan: every archetype of this world was
        // minted through a funnel that set the bit. Cold + one-time; one
        // Relaxed load (the panic is a config-time courtesy, not a soundness
        // fence).
        if component_registry::was_ever_archetyped(component_id.0) {
            register_component_hooks_stale_panic::<C>();
        }

        ComponentHooksBuilder::new(component_id.0)
    }

    // ── Phase 14b: component lifecycle observers (runtime-mutable) ──────────
    //
    // Unlike `register_component_hooks` (write-once per type, staleness-panics
    // if an archetype containing `C` already exists), observers are
    // runtime-mutable: `ArchetypeMaster::add_observer` runs the dynamic
    // add-first archetype walk, raising the `ON_{kind}_OBSERVER` bit on every
    // already-existing archetype containing `C`. There is therefore NO
    // staleness panic — late registration is handled by the walk.

    /// Registers an `on_add` observer for component `C`, returning a stable
    /// [`ObserverId`] for later [`Self::remove_observer`] (Phase 14b).
    ///
    /// The `runner` fires after the per-component `on_add` hook at every
    /// structural op that newly adds `C` to an entity. Observers are
    /// runtime-mutable, so this may be called even after archetypes containing
    /// `C` exist — the dynamic archetype walk raises the flag bit on them.
    #[inline]
    pub fn observe_on_add<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Add, C::component_id(), runner)
    }

    /// Registers an `on_insert` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). See [`Self::observe_on_add`] for semantics.
    #[inline]
    pub fn observe_on_insert<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Insert, C::component_id(), runner)
    }

    /// Registers an `on_replace` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). Fires before an existing `C` value is
    /// overwritten (and, on despawn, for the dying value). See
    /// [`Self::observe_on_add`].
    #[inline]
    pub fn observe_on_replace<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Replace, C::component_id(), runner)
    }

    /// Registers an `on_remove` observer for component `C`, returning a stable
    /// [`ObserverId`] (Phase 14b). Fires before `C` is removed from an entity
    /// (and, on despawn, for the dying value). See [`Self::observe_on_add`].
    #[inline]
    pub fn observe_on_remove<C: Component>(&mut self, runner: ObserverFn) -> ObserverId {
        self.archetype_master
            .add_observer(ObserverKind::Remove, C::component_id(), runner)
    }

    /// Registers `runner` as a `kind` observer for the component identified by
    /// `cid`, returning a stable [`ObserverId`] (Phase 14b).
    ///
    /// The type-erased sibling of the `observe_on_*::<C>` helpers: prefer those
    /// where the component type is statically known. This form is for callers
    /// that already hold a resolved [`ComponentId`].
    #[inline]
    pub fn add_observer(
        &mut self,
        kind: ObserverKind,
        cid: ComponentId,
        runner: ObserverFn,
    ) -> ObserverId {
        self.archetype_master.add_observer(kind, cid, runner)
    }

    /// Removes the observer with `id`, returning `true` if it was registered
    /// (Phase 14b).
    ///
    /// On removal of the last observer for its `(kind, component)` pair, the
    /// corresponding `ON_{kind}_OBSERVER` archetype flag bits are recomputed
    /// (cleared where no sibling component still observes that kind, hook bits
    /// preserved).
    #[inline]
    pub fn remove_observer(&mut self, id: ObserverId) -> bool {
        self.archetype_master.remove_observer(id)
    }

    // ── Feature 2: entity-targeted observers + custom triggers ──────────────

    /// Raises the STICKY [`ArchetypeFlags::HAS_ENTITY_OBSERVER`] bit on
    /// `entity`'s current archetype (FIX W2/C4/C5).
    ///
    /// Set-once, never cleared: runs under `&mut self` (no fire in flight), a
    /// single `|=` before any fire reads the flag. A no-op for a stale / dead
    /// entity handle (its archetype, if any, is left untouched).
    fn raise_entity_observer_bit(&mut self, entity: Entity) {
        // Copy the 16 B inland by value to release the `entity_master` borrow
        // before dereferencing the raw `archetype_ptr` (the established idiom —
        // the write targets the archetype slab, a disjoint allocation).
        let inland: EntityInland = match self.entity_master.entities_inland.get(entity.id().0) {
            Some(slot) => *slot,
            None => return,
        };
        if inland.is_null() || inland.generation() != entity.generation() {
            return;
        }
        let archetype_ptr = inland.archetype_ptr();
        // SAFETY (F1): `archetype_ptr` is the entity's stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance for the EcsMaster's
        //   lifetime. We run under `&mut self` (no concurrent reader), so this
        //   `|=` does not race a lockless flags read. The bit is sticky (never
        //   cleared), so no re-raise can lose a concurrent set.
        unsafe {
            (*archetype_ptr).flags.insert(ArchetypeFlags::HAS_ENTITY_OBSERVER);
        }
    }

    /// Re-raises the sticky `HAS_ENTITY_OBSERVER` bit on `entity`'s CURRENT
    /// (post-migration) archetype iff `entity` still has an entity-targeted
    /// observer (FIX W2/C4/C5 — the migration-to-a-new-archetype half).
    ///
    /// Called at the migration completion sites. Gated by the store's
    /// `has_observer` probe so it is a no-op (one `Option::is_none()`) for an
    /// entity with no entity observer — the 0%-gate. The bit on the SOURCE
    /// archetype is never cleared (sticky).
    pub(crate) fn migrate_entity_observer_bit(&mut self, entity: Entity) {
        if self.entity_observers.has_observer(entity) {
            self.raise_entity_observer_bit(entity);
        }
    }

    /// Attaches an entity-targeted lifecycle observer: fires only when `kind`
    /// happens to `cid` ON `entity`. Returns a stable [`ObserverId`].
    ///
    /// Raises the sticky `HAS_ENTITY_OBSERVER` bit on `entity`'s archetype so
    /// the fire sites probe the per-entity store for this archetype.
    ///
    /// # Live-entity contract
    ///
    /// `entity` MUST be LIVE (already spawned, not yet despawned). An
    /// entity-targeted lifecycle observer fires for events that happen to that
    /// live entity AFTER attachment:
    ///
    /// * `on_add` / `on_insert` fire when a component is LATER added or inserted
    ///   via the migration path — NOT retroactively for components already
    ///   present on `entity`, and NOT at the entity's initial spawn (the spawn
    ///   flow is spawn-THEN-observe).
    /// * `on_replace` / `on_remove` / `on_despawn` fire when the matching event
    ///   later happens to the live entity.
    ///
    /// Attaching to a reserved-but-not-yet-spawned or already-dead handle will
    /// NOT fire (a debug build asserts liveness; release silently ignores it,
    /// matching the rest of the stale-handle API).
    pub fn observe_entity(
        &mut self,
        entity: Entity,
        kind: ObserverKind,
        cid: ComponentId,
        runner: ObserverFn,
    ) -> ObserverId {
        debug_assert!(
            self.is_entity_live(entity),
            "observe_entity: entity is not live (live-entity contract) — an \
             entity-targeted observer must be attached to an already-spawned, \
             not-yet-despawned entity; it fires for events AFTER attachment, \
             never retroactively or at spawn"
        );
        let id = self
            .entity_observers
            .observe_entity_lifecycle(entity, kind, cid, runner);
        self.raise_entity_observer_bit(entity);
        id
    }

    /// Typed sugar: attach an `on_despawn` entity observer for component `C` on
    /// `entity` (the Feature-2 entity-level despawn callback).
    ///
    /// `entity` MUST be LIVE — see [`observe_entity`](Self::observe_entity)'s
    /// live-entity contract (the liveness `debug_assert!` is enforced there).
    #[inline]
    pub fn observe_entity_on_despawn<C: Component>(
        &mut self,
        entity: Entity,
        runner: ObserverFn,
    ) -> ObserverId {
        self.observe_entity(entity, ObserverKind::Despawn, C::component_id(), runner)
    }

    /// Registers a GLOBAL observer for custom trigger `E` (fires for any
    /// target). Returns a stable [`ObserverId`].
    pub fn observe<E: Trigger>(&mut self, runner: TriggerFn) -> ObserverId {
        let tid = Self::trigger_id::<E>();
        self.triggers.add(tid, runner)
    }

    /// Registers an ENTITY-TARGETED observer for custom trigger `E` on `entity`.
    /// Returns a stable [`ObserverId`]. Raises the sticky archetype bit.
    ///
    /// # Live-entity contract
    ///
    /// `entity` MUST be LIVE (already spawned, not yet despawned). The observer
    /// fires only for `trigger::<E>(entity, ..)` calls that target this live
    /// entity AFTER attachment (custom triggers are explicit, never retroactive
    /// and never raised at spawn). Attaching to a reserved-but-not-yet-spawned
    /// or already-dead handle will NOT fire (a debug build asserts liveness;
    /// release silently ignores it).
    pub fn observe_entity_event<E: Trigger>(
        &mut self,
        entity: Entity,
        runner: TriggerFn,
    ) -> ObserverId {
        debug_assert!(
            self.is_entity_live(entity),
            "observe_entity_event: entity is not live (live-entity contract) — \
             an entity-targeted trigger observer must be attached to an \
             already-spawned, not-yet-despawned entity; it fires only for \
             triggers raised at this entity AFTER attachment"
        );
        let tid = Self::trigger_id::<E>();
        let id = self.entity_observers.observe_entity_custom(entity, tid, runner);
        self.raise_entity_observer_bit(entity);
        id
    }

    // ── Relation-edge observers: OnLink<R> / OnUnlink<R> (Decision 5) ───────

    /// Registers a GLOBAL observer for the relation-edge trigger
    /// [`OnLink<R>`](crate::ecs::core::relationship::OnLink): fires whenever an
    /// `R` edge is COMMITTED (a new foreign key, or the new side of a
    /// re-target), targeting the source entity. The runner reads the committed
    /// `target` from the `OnLink<R>` event.
    ///
    /// A thin wrapper over [`observe`](Self::observe) keyed on `OnLink<R>`'s
    /// dense trigger id (no new dispatch path — the `(R, *)` analogue).
    #[inline]
    pub fn observe_on_link<R: Relationship>(&mut self, runner: TriggerFn) -> ObserverId {
        self.observe::<OnLink<R>>(runner)
    }

    /// Registers a GLOBAL observer for the relation-edge trigger
    /// [`OnUnlink<R>`](crate::ecs::core::relationship::OnUnlink): fires whenever
    /// an `R` edge is DESTROYED (an explicit remove, the old side of a
    /// re-target, a source despawn, or a non-cascading target teardown),
    /// targeting the source entity. The runner reads the destroyed `old_target`
    /// from the `OnUnlink<R>` event.
    #[inline]
    pub fn observe_on_unlink<R: Relationship>(&mut self, runner: TriggerFn) -> ObserverId {
        self.observe::<OnUnlink<R>>(runner)
    }

    /// `true` iff ANY observer (global or entity-targeted) listens for the
    /// trigger id `tid`. The cold 0%-probe for the edge-fire sites: one
    /// global-registry read + one entity-store sticky-aggregate read, both
    /// lazy-`None`-gated, so a world with no edge observers pays ~nothing.
    #[inline]
    pub(crate) fn has_edge_observer(&self, tid: TriggerId) -> bool {
        self.triggers.has(tid) || self.entity_observers.has_any_custom(tid)
    }

    /// Fires [`OnLink<R>`](crate::ecs::core::relationship::OnLink) on the source
    /// of a freshly-COMMITTED `R` edge, gated behind the cold 0%-probe.
    ///
    /// Called from
    /// [`LinkCommand::apply`](crate::ecs::core::relationship::LinkCommand) AFTER
    /// the dangling-target guard, under `&mut EcsMaster` at the apply window —
    /// the synchronous `trigger` walk is sound there (it re-enters the audited
    /// command drain on a separate allocation; W3 fence preserved because the
    /// hook bodies only ENQUEUE, never fire).
    #[inline]
    pub(crate) fn fire_on_link<R: Relationship>(&mut self, source: Entity, target: Entity) {
        let tid = Self::trigger_id::<OnLink<R>>();
        if self.has_edge_observer(tid) {
            self.fire_edge_observer::<OnLink<R>>(tid, source, OnLink::<R>::new(target));
        }
    }

    /// Fires [`OnUnlink<R>`](crate::ecs::core::relationship::OnUnlink) on the
    /// source of a freshly-DESTROYED `R` edge, gated behind the cold 0%-probe.
    ///
    /// Called from
    /// [`UnlinkCommand::apply`](crate::ecs::core::relationship::UnlinkCommand)
    /// only when the source was actually present in the target's reverse
    /// collection (the committed-edge test), under `&mut EcsMaster`.
    #[inline]
    pub(crate) fn fire_on_unlink<R: Relationship>(&mut self, source: Entity, target: Entity) {
        let tid = Self::trigger_id::<OnUnlink<R>>();
        if self.has_edge_observer(tid) {
            self.fire_edge_observer::<OnUnlink<R>>(tid, source, OnUnlink::<R>::new(target));
        }
    }

    /// Drives the synchronous trigger walk for an edge event — a `#[cold]`
    /// out-of-line tail so the gated common case (no edge observer) keeps the
    /// committed-edge `apply` body compact (I-cache).
    #[cold]
    #[inline(never)]
    fn fire_edge_observer<E: Trigger>(&mut self, tid: TriggerId, source: Entity, event: E) {
        self.trigger_walk::<E>(tid, source, &event);
    }

    /// Fires a custom trigger at `target`: runs global observers for `E`, then
    /// entity-targeted observers for `target`, then propagates per
    /// `E::PROPAGATION` ([`Up`](PropagationMode::Up) bubble or
    /// [`Down`](PropagationMode::Down) broadcast).
    ///
    /// `event` is moved in and lives on this frame until the walk ends; runners
    /// read it through a read-only `*const u8` and cannot move or free it.
    pub fn trigger<E: Trigger>(&mut self, target: Entity, event: E) {
        let tid = Self::trigger_id::<E>();
        self.trigger_walk::<E>(tid, target, &event);
    }

    /// Fires a global-only (untargeted) custom trigger — runs only the global
    /// observers for `E` (no entity targeting, no propagation).
    pub fn trigger_global<E: Trigger>(&mut self, event: E) {
        let tid = Self::trigger_id::<E>();
        let world_ptr = NonNull::from(&mut *self);
        // A sentinel target: `trigger_global` never reads it (global-only). The
        // ctx is required by the shared TriggerFn shape.
        let ctx = TriggerContext {
            target: Entity::new(EntityId(usize::MAX), 0),
            original_target: Entity::new(EntityId(usize::MAX), 0),
            trigger_id: tid,
        };
        fire_global_triggers(world_ptr, tid, ctx, (&event as *const E).cast());
    }

    /// Removes any Feature-2 observer (entity-targeted lifecycle/custom or
    /// global trigger) by its id, returning `true` if it was registered.
    ///
    /// Does NOT clear the sticky `HAS_ENTITY_OBSERVER` bit (set-once forever).
    pub fn remove_observer_any(&mut self, id: ObserverId) -> bool {
        self.entity_observers.remove(id) || self.triggers.remove(id)
    }

    /// Returns the process-stable dense [`TriggerId`] for `E`, cached per type.
    #[inline]
    fn trigger_id<E: Trigger>() -> TriggerId {
        static_trigger_id::<E>()
    }

    /// The custom-trigger fire + propagation walk (Feature 2 algorithm B,
    /// extended with the `Down` broadcast — Decision 6).
    ///
    /// Re-derives all `world`-borrows per turn (OBS-FIRE-LOOP); the propagation
    /// `propagate` bool lives in TLS via [`PropagateGuard`] (FIX W9). `target` /
    /// `original_target` travel in [`TriggerContext`] BY VALUE.
    ///
    /// Branches on `E::PROPAGATION` (const-folded): the
    /// [`None`](PropagationMode::None) / [`Up`](PropagationMode::Up) arm is the
    /// byte-identical pre-broadcast linear walk; the
    /// [`Down`](PropagationMode::Down) arm is the relation-aware fan-out
    /// (`trigger_broadcast_down`). For a non-`Down` trigger the `Down` call site
    /// const-folds away entirely (the 0%-gate at the type level — existing `Up`
    /// / `None` triggers keep their exact code generation).
    fn trigger_walk<E: Trigger>(&mut self, tid: TriggerId, target: Entity, event: &E) {
        let event_ptr: *const u8 = (event as *const E).cast();
        let original = target;
        // Save/restore the propagation TLS across this (possibly re-entrant)
        // walk; seed it with the event's compile-time AUTO_PROPAGATE.
        let _guard = PropagateGuard::enter(E::AUTO_PROPAGATE);

        if const { matches!(E::PROPAGATION, PropagationMode::Down) } {
            // Relation-aware DOWNWARD broadcast: fire on `target`, then DFS
            // `E::Broadcast`'s reverse collection, per-node propagate snapshot.
            self.trigger_broadcast_down::<E>(tid, original, event_ptr);
            return;
        }

        // ── None / Up: the byte-identical pre-broadcast linear walk ─────────
        let mut current = target;
        let mut hops = 0usize;
        loop {
            let ctx = TriggerContext { target: current, original_target: original, trigger_id: tid };
            // Probe the sticky bit FIRST (a `&self` read), BEFORE minting any raw
            // `world_ptr`, so no shared reborrow spans a raw-pointer use (F2).
            let has_entity_obs = self.entity_archetype_has_entity_observer(current);
            // Global observers — mint `world_ptr` fresh immediately before use.
            fire_global_triggers(NonNull::from(&mut *self), tid, ctx, event_ptr);
            // Entity-targeted observers for the current target, gated by the
            // archetype's sticky HAS_ENTITY_OBSERVER bit. Re-mint `world_ptr`.
            if has_entity_obs {
                fire_entity_triggers(NonNull::from(&mut *self), tid, ctx, event_ptr);
            }
            // FIX F1: decide whether to bubble purely from the propagation TLS.
            // `PropagateGuard::enter(E::AUTO_PROPAGATE)` (above) SEEDED the TLS
            // with the compile-time `AUTO_PROPAGATE` constant, so the const-fold
            // of the non-bubbling case lives in the seed — NOT in this condition.
            // Reading only `get_propagate()` keeps both directions correct:
            //   * a bubbling event (seed `true`) keeps walking until an observer
            //     calls `propagate(false)` to STOP it (the prior `const { .. } ||`
            //     short-circuit elided this read, making `propagate(false)` a
            //     silent no-op);
            //   * a non-bubbling event (seed `false`) breaks after this single
            //     hop unless an observer opted in with `propagate(true)`.
            if !get_propagate() {
                break;
            }
            hops += 1;
            debug_assert!(
                hops < crate::ecs::constants::MAX_PROPAGATION_DEPTH,
                "trigger propagation exceeded MAX_PROPAGATION_DEPTH ({}) — ChildOf cycle?",
                crate::ecs::constants::MAX_PROPAGATION_DEPTH
            );
            // Re-derive the next hop through a fresh read-only view (no `&` spans
            // the next fire). The view is minted and dropped within this block.
            let next = {
                // SAFETY (`DeferredEcsMaster::from_world` contract): `&mut *self`
                //   is the live, exclusively-held world; no `world`-derived
                //   `&mut Archetype`/`&mut ComponentPool` is live at this mint
                //   point (the fires above dropped theirs); and we are inside the
                //   single-threaded apply window. The view is read-only and dies
                //   at the end of this block before the next fire reborrows.
                let view = unsafe { DeferredEcsMaster::from_world(NonNull::from(&mut *self)) };
                E::Traversal::next(&view, current)
            };
            match next {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }

    /// Fires the GLOBAL + entity-targeted custom-trigger observers for `tid` at
    /// ONE node (`current`), the per-node fire used by the `Down` broadcast.
    ///
    /// Mirrors the per-node fire of the linear walk exactly: probe the sticky
    /// bit first (`&self`), fire global observers (re-mint `world_ptr`), then —
    /// if the archetype observes — fire entity-targeted observers (re-mint).
    /// No `world`-derived `&` spans a raw-pointer use (F2 / OBS-FIRE-LOOP).
    #[inline]
    fn fire_node_triggers(
        &mut self,
        tid: TriggerId,
        current: Entity,
        original: Entity,
        event_ptr: *const u8,
    ) {
        let ctx = TriggerContext { target: current, original_target: original, trigger_id: tid };
        let has_entity_obs = self.entity_archetype_has_entity_observer(current);
        fire_global_triggers(NonNull::from(&mut *self), tid, ctx, event_ptr);
        if has_entity_obs {
            fire_entity_triggers(NonNull::from(&mut *self), tid, ctx, event_ptr);
        }
    }

    /// DOWNWARD broadcast walk (Decision 6 / critic W4): fires `E` on `root`,
    /// then recursively over every source in `E::Broadcast`'s reverse
    /// collection — an explicit-stack DFS that reuses the EXACT depth-cap +
    /// `!ACYCLIC` visited discipline of the query-side `DescendantsIter`, with a
    /// PER-NODE propagate snapshot so a `propagate(false)` from one node's
    /// observer prunes ONLY that node's subtree (never a sibling's).
    ///
    /// # Per-node propagate snapshot (critic W4)
    ///
    /// The linear `Up` bubble is a single chain, so one propagate `Cell`
    /// suffices. A `Down` DFS has many live sibling subtrees, so a global flag
    /// would let node X's `propagate(false)` leak to sibling Y. Each node's fire
    /// is wrapped in a snapshot: seed `true` (fan-out-all default), fire, read
    /// the post-fire flag (the prune decision for THIS node's children), then
    /// RESTORE the caller's flag before moving to the next sibling. A node is
    /// expanded (its children pushed) iff its own post-fire flag stayed `true`.
    ///
    /// # Cycle + depth safety
    ///
    /// Bounded by [`MAX_PROPAGATION_DEPTH`](crate::ecs::constants::MAX_PROPAGATION_DEPTH).
    /// For a non-`ACYCLIC` `E::Broadcast` a `#[cold]` function-local visited set
    /// keeps each node visited at most once; for an `ACYCLIC` relation (e.g.
    /// `ChildOf`) the visited guard const-folds away and the depth cap alone
    /// bounds the walk — identical to `DescendantsIter`.
    #[cold]
    #[inline(never)]
    fn trigger_broadcast_down<E: Trigger>(
        &mut self,
        tid: TriggerId,
        root: Entity,
        event_ptr: *const u8,
    ) {
        use crate::ecs::constants::MAX_PROPAGATION_DEPTH;

        // Fire on the root first (depth 0). Snapshot/seed-true/restore so the
        // root's `propagate(false)` prunes the whole broadcast (no descent).
        let saved = get_propagate();
        propagate(true);
        self.fire_node_triggers(tid, root, root, event_ptr);
        let expand_root = get_propagate();
        propagate(saved);
        if !expand_root {
            return;
        }

        // Explicit-stack DFS frontier of `(node, depth)`, transient scratch
        // (function-local — NOT a durable side store, Principle 0). Seed with the
        // root's direct sources at depth 1.
        let mut stack: Vec<(Entity, usize)> = Vec::new();
        let mut visited = VisitedSet::default();
        if const { !<E::Broadcast as Relationship>::ACYCLIC } {
            visited.insert_seen(root.id().0);
        }
        self.push_broadcast_sources::<E>(root, 1, &mut stack);

        while let Some((node, depth)) = stack.pop() {
            debug_assert!(
                depth <= MAX_PROPAGATION_DEPTH,
                "trigger Down broadcast exceeded MAX_PROPAGATION_DEPTH"
            );
            // `!ACYCLIC`: skip a node already fired (the ≤ C·N guarantee + cycle
            // termination). Const-folds away for an acyclic broadcast relation.
            if const { !<E::Broadcast as Relationship>::ACYCLIC }
                && visited.insert_seen(node.id().0)
            {
                continue;
            }
            // PER-NODE propagate snapshot (W4): seed true (fan-out-all), fire,
            // read the prune decision for THIS node's subtree, then restore.
            let saved = get_propagate();
            propagate(true);
            self.fire_node_triggers(tid, node, root, event_ptr);
            let expand = get_propagate();
            propagate(saved);
            // Descend only if this node did not prune itself AND we are below the
            // depth cap. A pruned node still counts as fired (it was), but its
            // subtree is skipped — exactly the sibling-isolating semantics.
            if expand && depth < MAX_PROPAGATION_DEPTH {
                self.push_broadcast_sources::<E>(node, depth + 1, &mut stack);
            }
        }
    }

    /// Pushes every source pointing at `node` through `E::Broadcast`'s reverse
    /// collection onto the DFS `stack` at `depth` (the `Down` broadcast fan-out
    /// step). Reuses the existing reverse index by O(1) index — the same hop
    /// `DescendantsIter::push_sources` pays. Re-derives the read-only view per
    /// call so no `world`-derived `&` spans a fire (OBS-FIRE-LOOP).
    #[inline]
    fn push_broadcast_sources<E: Trigger>(
        &self,
        node: Entity,
        depth: usize,
        stack: &mut Vec<(Entity, usize)>,
    ) {
        // The reverse-index component on the broadcast relation's target side.
        let Some(reverse) =
            self.get_component::<<E::Broadcast as Relationship>::Target>(node)
        else {
            return;
        };
        let collection = reverse.collection();
        let len = collection.len();
        for i in 0..len {
            if let Some(source) = collection.get(i) {
                stack.push((source, depth));
            }
        }
    }

    /// `true` iff `entity`'s current archetype has the sticky
    /// `HAS_ENTITY_OBSERVER` bit set. A stale / dead handle returns `false`.
    #[inline]
    fn entity_archetype_has_entity_observer(&self, entity: Entity) -> bool {
        let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        if slot.is_null() || slot.generation() != entity.generation() {
            return false;
        }
        // SAFETY (F1): stable, interior-mutable slab provenance; `&self` shared
        //   read of a `u16` flag (no `&mut` taken).
        unsafe { (*slot.archetype_ptr()).flags.contains(ArchetypeFlags::HAS_ENTITY_OBSERVER) }
    }

    /// `true` iff `entity` is currently LIVE: its `entities_inland` slot is
    /// resolvable, non-null (spawned, not a reserved-only handle), and its
    /// generation matches (not despawned / recycled). Used by the Feature-2
    /// `observe_entity*` attach paths to enforce the live-entity contract via a
    /// debug-only assertion (see [`observe_entity`](Self::observe_entity)).
    #[inline]
    fn is_entity_live(&self, entity: Entity) -> bool {
        match self.entity_master.entities_inland.get(entity.id().0) {
            Some(slot) => !slot.is_null() && slot.generation() == entity.generation(),
            None => false,
        }
    }

}

/// Cold-path panic helper for [`EcsMaster::register_component_hooks`] when the
/// release-level staleness gate finds `C` already placed in an archetype (plan
/// §6.4 / Q-A5 / W3; Phase 21 H1 — the gate is process-global across all
/// worlds). Kept off the hot method body via `#[cold] #[inline(never)]`.
#[cold]
#[inline(never)]
fn register_component_hooks_stale_panic<C: Component>() -> ! {
    panic!(
        "register_component_hooks::<{}>() called after {} already appears in a live \
         archetype of some world in this process (hooks are process-global per type, \
         so the gate is too); register hooks before the component is first used in ANY \
         world (the archetype's ArchetypeFlags were computed at construction and would \
         be stale, silently skipping the hook).",
        C::debug_type_name(),
        C::debug_type_name(),
    );
}

/// Cold-path panic helper for [`EcsMaster::register_component_hooks`] when `C`
/// already declares `#[component(...)]` derive hooks (Wave-5 soundness fix /
/// Change 3 — derive XOR runtime). Eager check at the registration call site,
/// kept off the method body via `#[cold] #[inline(never)]`.
#[cold]
#[inline(never)]
fn register_component_hooks_derive_conflict_panic<C: Component>() -> ! {
    panic!(
        "register_component_hooks::<{}>() on a type that declares #[component(...)] \
         derive hooks — use the derive OR the runtime builder, not both.",
        C::debug_type_name(),
    );
}
