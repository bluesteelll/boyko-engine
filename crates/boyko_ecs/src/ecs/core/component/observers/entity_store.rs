//! `EntityObserverStore` — per-world entity-targeted observer storage
//! (Feature 2 D1).
//!
//! An entity-targeted observer fires only when an event happens to ONE specific
//! [`Entity`] — a lifecycle [`ObserverKind`] or a custom trigger. Entities with
//! no entity-targeted observer are ABSENT from the store and pay nothing; the
//! zero-cost gate is the
//! [`ArchetypeFlags::HAS_ENTITY_OBSERVER`](crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags::HAS_ENTITY_OBSERVER)
//! sticky bit on
//! the entity's archetype (set once, never cleared — FIX W2/C4/C5), so the
//! sparse probe is skipped entirely for archetypes that have never observed.
//!
//! # Storage shape (FIX W8)
//!
//! [`SparseMap`] is `impl<U: Clone>` and its `swap_remove` deep-clones the value
//! on the non-last path, so storing the per-entity list directly would clone the
//! whole entries `Vec` on every detach/despawn (a hidden alloc). Instead the map
//! stores a `u32` HANDLE (Copy — cheap `swap_remove`) into a side
//! `Vec<EntityObserverList>` arena with a free-list; detach `mem::take`s the
//! arena slot (no clone), and the freed handle is recycled.
//!
//! # Soundness (OBS-FIRE-LOOP / F2)
//!
//! [`fire_entity_observers`] re-derives `&world.entity_observers` per turn,
//! copies the matching [`EntityRunner`] out by value, and drops the `&` BEFORE
//! minting [`DeferredEcsMaster`]. No `world`-derived `&` spans the view mint or
//! the runner call (the F2 / 9.3c Tree-Borrows discipline).
//!
//! # Runner storage (FIX F3)
//!
//! The runner is stored as a real fn-pointer in an [`EntityRunner`] tagged
//! union, NOT as a `usize`. Under `-Zmiri-tree-borrows` strict provenance,
//! casting a fn-ptr to an integer strips its provenance; transmuting the bare
//! integer back to a fn-ptr and calling it is UB ("dangling pointer (no
//! provenance)"). Keeping a typed fn-ptr preserves provenance through the whole
//! attach → fire round-trip with no `as usize` and no `transmute`.

use std::ptr::NonNull;

use boyko_utils::sparse_map::sparse_map::SparseMap;

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::component::observers::dispatch_key::DispatchKey;
use crate::ecs::core::component::observers::trigger::{TriggerContext, TriggerFn};
use crate::ecs::core::component::observers::{
    ObserverContext, ObserverFn, ObserverId, ObserverKind,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// The runner of one entity-targeted observer, as a REAL fn-pointer (FIX F3).
///
/// A lifecycle observer stores an [`ObserverFn`]; a custom-trigger observer
/// stores a [`TriggerFn`]. Both are 8-byte `unsafe fn(…)` pointers, so the enum
/// is `Copy + Send + Sync`. The variant always corresponds to the entry's
/// [`DispatchKey::is_custom`] bit — the fire site `match`es on the variant
/// directly (no `as usize`, no `transmute`), preserving fn-ptr provenance for
/// Tree Borrows.
#[derive(Clone, Copy)]
enum EntityRunner {
    /// A lifecycle observer's runner.
    Lifecycle(ObserverFn),
    /// A custom-trigger observer's runner.
    Custom(TriggerFn),
}

/// One registered entity-targeted observer.
///
/// Carries the [`DispatchKey`] (lifecycle kind / custom trigger), the optional
/// component scope (for lifecycle observers — a sentinel for custom triggers),
/// the stable id, and the [`EntityRunner`]. The runner variant always matches
/// the key's [`DispatchKey::is_custom`] bit.
#[derive(Clone, Copy)]
struct EntityObserverEntry {
    /// What this observer listens for (lifecycle kind or custom trigger).
    key: DispatchKey,
    /// Component scope for lifecycle observers; ignored for custom triggers.
    component_id: ComponentId,
    /// Stable id (matched on removal).
    id: ObserverId,
    /// The runner, as a typed fn-pointer tagged by lifecycle vs custom.
    runner: EntityRunner,
}

/// One entity's observers, keyed by the unified [`DispatchKey`].
///
/// `Default` so a freed arena slot can be `mem::take`-n cheaply.
#[derive(Default)]
struct EntityObserverList {
    /// The `EntityId.0` this slot belongs to, so `remove` can prune the
    /// `by_entity` mapping when it empties the list (the arena has no other
    /// back-reference). Meaningful only when `occupied`.
    entity_id: usize,
    /// The generation the entity had when its first observer was attached. A
    /// despawn+reuse bumps the live generation; a stale list whose generation no
    /// longer matches is reclaimed and never fires (the recycle guard).
    generation: u32,
    /// Whether this arena slot is currently occupied (vs a free-list hole).
    occupied: bool,
    /// `(key, entry)` pairs — small-N, scanned linearly (cache-friendly).
    entries: Vec<EntityObserverEntry>,
}

/// Per-world entity-targeted observer store.
///
/// Lazy: `inner` stays `None` until the first `observe_entity*`, so a world that
/// never attaches an entity observer pays one `Option::is_none()` per relevant
/// site.
pub(crate) struct EntityObserverStore {
    inner: Option<Box<EntityObserverInner>>,
    /// Monotonic id source, outside the `Box` so it is valid before any
    /// allocation. Shared id space with the trigger registry is not required —
    /// ids are only ever matched within this store.
    next_id: u64,
}

struct EntityObserverInner {
    /// `EntityId.0 -> arena handle`. Copy payload (`u32`) ⇒ cheap `swap_remove`.
    by_entity: SparseMap<u32>,
    /// Side arena of per-entity lists, indexed by the handle stored in
    /// `by_entity`. Slots are recycled through `free_list`.
    arena: Vec<EntityObserverList>,
    /// Free arena slots (handles) available for reuse.
    free_list: Vec<u32>,
    /// STICKY per-`TriggerId` "has any entity ever attached a custom-trigger
    /// observer for this id" flag (set-once, never cleared). Backs the
    /// [`has_any_custom`](EntityObserverStore::has_any_custom) 0%-probe for the
    /// relation-edge observers. Set-once (the sticky-archetype-bit precedent):
    /// a world that never attaches an entity edge observer keeps every bit
    /// clear (true 0%-gate); a world that attaches then removes one keeps the
    /// bit set, so the probe stays conservatively `true` and the edge `trigger`
    /// walk runs but finds nothing (a rare, correctness-preserving no-op — it
    /// never MISSES a live observer, which a decrementing ref-count could on a
    /// `retire`/recycle path). Lazily grown, one `bool` per minted trigger id.
    ever_custom: Vec<bool>,
}

impl EntityObserverStore {
    /// Creates an empty store — zero allocation.
    #[inline]
    pub(crate) fn new() -> Self {
        Self { inner: None, next_id: 0 }
    }

    /// Allocates the inner storage on first use.
    #[inline]
    fn inner_mut(&mut self) -> &mut EntityObserverInner {
        self.inner.get_or_insert_with(|| {
            Box::new(EntityObserverInner {
                by_entity: SparseMap::new(),
                arena: Vec::new(),
                free_list: Vec::new(),
                ever_custom: Vec::new(),
            })
        })
    }

    /// Mints the next stable [`ObserverId`].
    #[inline]
    fn mint_id(&mut self) -> ObserverId {
        let id = ObserverId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Attaches an entity-targeted observer for `entity`, returning its stable
    /// id. Internal helper shared by the lifecycle / custom entry points; the
    /// caller (`EcsMaster`) is responsible for raising the sticky archetype bit.
    ///
    /// If `entity`'s slot holds a list from a PRIOR generation (a recycled
    /// `EntityId`), the stale list is reset before the new entry is appended —
    /// the recycle guard.
    fn attach(
        &mut self,
        entity: Entity,
        key: DispatchKey,
        component_id: ComponentId,
        runner: EntityRunner,
    ) -> ObserverId {
        let id = self.mint_id();
        let eid = entity.id().0;
        let generation = entity.generation();
        let inner = self.inner_mut();

        let handle = match inner.by_entity.get(eid).copied() {
            Some(h) => h,
            None => {
                // Allocate a fresh arena slot (recycle a free one if available).
                let h = match inner.free_list.pop() {
                    Some(h) => h,
                    None => {
                        let h = inner.arena.len() as u32;
                        inner.arena.push(EntityObserverList::default());
                        h
                    }
                };
                inner.by_entity.insert(eid, h);
                h
            }
        };

        let slot = &mut inner.arena[handle as usize];
        if !slot.occupied || slot.generation != generation {
            // Fresh attach, or a recycled EntityId from a dead generation:
            // reset and re-seed the generation (recycle guard).
            slot.entries.clear();
            slot.entity_id = eid;
            slot.generation = generation;
            slot.occupied = true;
        }
        slot.entries.push(EntityObserverEntry { key, component_id, id, runner });
        id
    }

    /// Attaches a lifecycle entity observer (`kind` on `component_id` for
    /// `entity`).
    #[inline]
    pub(crate) fn observe_entity_lifecycle(
        &mut self,
        entity: Entity,
        kind: ObserverKind,
        component_id: ComponentId,
        runner: ObserverFn,
    ) -> ObserverId {
        self.attach(
            entity,
            DispatchKey::lifecycle(kind),
            component_id,
            EntityRunner::Lifecycle(runner),
        )
    }

    /// Attaches a custom-trigger entity observer (`trigger_id` for `entity`).
    #[inline]
    pub(crate) fn observe_entity_custom(
        &mut self,
        entity: Entity,
        trigger_id: u32,
        runner: TriggerFn,
    ) -> ObserverId {
        let id = self.attach(
            entity,
            DispatchKey::custom(trigger_id),
            ComponentId(0),
            EntityRunner::Custom(runner),
        );
        // Raise the sticky `ever_custom` flag for this id (set-once, never
        // cleared) so the `has_any_custom` 0%-probe sees it.
        let inner = self.inner_mut();
        let tid = trigger_id as usize;
        if tid >= inner.ever_custom.len() {
            inner.ever_custom.resize(tid + 1, false);
        }
        inner.ever_custom[tid] = true;
        id
    }

    /// `true` iff ANY entity has EVER attached a custom-trigger observer for
    /// `trigger_id` (sticky, never cleared — see
    /// [`ever_custom`](EntityObserverInner::ever_custom)).
    ///
    /// The entity-store half of the relation-edge observers' cold 0%-probe: a
    /// world that never attaches an entity edge observer takes the lazy-`None`
    /// early-out. Conservative by design (it can return `true` after the last
    /// such observer is removed), which keeps the gate sound — it never reports
    /// `false` while a live observer exists.
    #[inline]
    pub(crate) fn has_any_custom(&self, trigger_id: u32) -> bool {
        self.inner
            .as_ref()
            .and_then(|i| i.ever_custom.get(trigger_id as usize))
            .copied()
            .unwrap_or(false)
    }

    /// Removes the observer with `id`, returning `true` if it was registered.
    ///
    /// Empty lists are reclaimed: the entity's `by_entity` entry is dropped and
    /// the arena slot is `mem::take`-n back onto the free-list (no clone — FIX
    /// W8). The sticky archetype bit is NOT cleared (it is set-once forever).
    pub(crate) fn remove(&mut self, id: ObserverId) -> bool {
        let Some(inner) = self.inner.as_mut() else {
            return false;
        };
        // Linear scan over occupied arena slots (entity observers are sparse).
        for handle in 0..inner.arena.len() {
            let slot = &mut inner.arena[handle];
            if !slot.occupied {
                continue;
            }
            if let Some(pos) = slot.entries.iter().position(|e| e.id == id) {
                slot.entries.swap_remove(pos);
                if slot.entries.is_empty() {
                    // Last observer for this entity removed: drop the `by_entity`
                    // mapping (so `retire` never double-frees the handle) and
                    // return the arena slot to the free-list. `entity_id` is the
                    // arena's back-reference to the map key.
                    let eid = slot.entity_id;
                    let h = handle as u32;
                    *slot = EntityObserverList::default();
                    inner.by_entity.swap_remove(eid);
                    inner.free_list.push(h);
                }
                return true;
            }
        }
        false
    }

    /// `true` iff `entity` currently has ≥1 live entity-targeted observer.
    ///
    /// Used by the migration sites to decide whether to re-raise the sticky
    /// `HAS_ENTITY_OBSERVER` bit on the destination archetype. Cheap: one lazy
    /// `Option` check + one `SparseMap` probe + a generation compare.
    #[inline]
    pub(crate) fn has_observer(&self, entity: Entity) -> bool {
        let Some(inner) = self.inner.as_ref() else {
            return false;
        };
        let Some(handle) = inner.by_entity.get(entity.id().0) else {
            return false;
        };
        let slot = &inner.arena[*handle as usize];
        slot.occupied && slot.generation == entity.generation() && !slot.entries.is_empty()
    }

    /// Retires `entity`'s observer slot on despawn (or generation bump): drops
    /// the `by_entity` mapping and returns the arena slot to the free-list.
    ///
    /// Idempotent — a no-op when `entity` has no observers. The sticky archetype
    /// bit is never cleared.
    pub(crate) fn retire(&mut self, entity: Entity) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let eid = entity.id().0;
        if let Some(handle) = inner.by_entity.swap_remove(eid) {
            let slot = &mut inner.arena[handle as usize];
            // `mem::take` resets to `Default` (no clone — FIX W8).
            *slot = EntityObserverList::default();
            inner.free_list.push(handle);
        }
    }

    /// Returns the runner of the `i`-th entity observer of `entity` matching
    /// `key` (and `component_filter`, when `Some`), skipping stale-generation
    /// lists — or `None` when the index is past the end.
    ///
    /// Used by the fire loops: each re-derives a fresh `&self` per turn, so this
    /// returns the runner COPIED OUT by value (no borrow escapes). A `Some`
    /// `component_filter` narrows lifecycle matches to one `(kind, component)`
    /// pair; `None` matches every entry with `key` (custom triggers, which have
    /// no component scope).
    #[inline]
    fn lookup_nth(
        &self,
        entity: Entity,
        key: DispatchKey,
        component_filter: Option<ComponentId>,
        i: usize,
    ) -> Option<EntityRunner> {
        let inner = self.inner.as_ref()?;
        let handle = *inner.by_entity.get(entity.id().0)?;
        let slot = &inner.arena[handle as usize];
        if !slot.occupied || slot.generation != entity.generation() {
            return None;
        }
        slot.entries
            .iter()
            .filter(|e| {
                e.key == key && component_filter.is_none_or(|c| e.component_id == c)
            })
            .nth(i)
            .map(|e| e.runner)
    }
}

impl Default for EntityObserverStore {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// Fires every LIFECYCLE entity-targeted observer registered for
/// `(kind, component_id)` on `entity` (Feature 2, OBS-FIRE-LOOP).
///
/// Cold: invoked only when the archetype's `HAS_ENTITY_OBSERVER`
/// bit is set (the caller's gate). Re-derives `&world.entity_observers` per turn,
/// copies the runner out by value, drops the `&` BEFORE minting the view.
#[cold]
#[inline(never)]
pub(crate) fn fire_entity_observers(
    world: NonNull<EcsMaster>,
    kind: ObserverKind,
    component_id: ComponentId,
    entity: Entity,
) {
    let key = DispatchKey::lifecycle(kind);
    let mut i = 0usize;
    loop {
        // Re-derive a FRESH, SHORT-LIVED store `&` each turn; copy the matching
        // runner out by value; drop the `&` at this block's close — BEFORE the
        // view mint (the OBS-FIRE-LOOP discipline).
        let runner = {
            // SAFETY (OBS-FIRE / F2): `world` was minted via `NonNull::from(&mut
            //   *self)` at the call site AFTER every `world`-derived `&mut` was
            //   dropped, so this shared read aliases no live reborrow. The `&` is
            //   re-derived per iteration and dropped at this block boundary,
            //   BEFORE any `world`-derived view is minted. Single-threaded apply
            //   window; the store is mutated only via `&mut self`, which cannot
            //   be live here.
            let store = unsafe { &world.as_ref().entity_observers };
            let Some(runner) = store.lookup_nth(entity, key, Some(component_id), i) else {
                break;
            };
            runner
        };
        // FIX F3: a lifecycle key (`key.is_custom() == false`) was attached via
        // `EntityRunner::Lifecycle`, so the variant matches the key bit.
        let EntityRunner::Lifecycle(runner) = runner else {
            debug_assert!(
                !key.is_custom(),
                "lifecycle fire matched a custom-runner entry (key/runner desync)"
            );
            unreachable!("a lifecycle key only matches EntityRunner::Lifecycle entries")
        };
        // No store `&` is live past this point.
        // SAFETY (SAFETY-1 / SAFETY-4): `world` aliases no live reborrow at the
        //   mint; firing happens only in the single-threaded apply window; the
        //   read-only view withholds every structural + `&mut`-into-pool method.
        let view = unsafe { DeferredEcsMaster::from_world(world) };
        // SAFETY (OBS-FIRE-CALL): the `ObserverFn` contract == apply-window +
        //   non-aliasing, exactly what the `unsafe fn` requires of its caller.
        unsafe {
            runner(view, ObserverContext { entity, component_id, kind });
        }
        i += 1;
    }
}

/// Fires every CUSTOM-trigger entity-targeted observer registered for
/// `trigger_id` on `ctx.target` (Feature 2, OBS-FIRE-LOOP).
///
/// Same re-derive discipline as [`fire_entity_observers`]; the runner is a
/// [`TriggerFn`] and receives the event by `*const u8` + the [`TriggerContext`].
#[cold]
#[inline(never)]
pub(crate) fn fire_entity_triggers(
    world: NonNull<EcsMaster>,
    trigger_id: u32,
    ctx: TriggerContext,
    event: *const u8,
) {
    let key = DispatchKey::custom(trigger_id);
    let mut i = 0usize;
    loop {
        let runner = {
            // SAFETY: identical re-derive discipline as `fire_entity_observers`.
            let store = unsafe { &world.as_ref().entity_observers };
            let Some(runner) = store.lookup_nth(ctx.target, key, None, i) else {
                break;
            };
            runner
        };
        // FIX F3: a custom key (`key.is_custom() == true`) was attached via
        // `EntityRunner::Custom`, so the variant matches the key bit.
        let EntityRunner::Custom(runner) = runner else {
            debug_assert!(
                key.is_custom(),
                "custom-trigger fire matched a lifecycle-runner entry (key/runner desync)"
            );
            unreachable!("a custom key only matches EntityRunner::Custom entries")
        };
        // SAFETY (SAFETY-1 / SAFETY-4): `world` aliases no live reborrow at the
        //   mint; single-threaded apply window; the read-only view withholds
        //   every structural + `&mut`-into-pool method.
        let view = unsafe { DeferredEcsMaster::from_world(world) };
        // SAFETY (TriggerFn contract): apply-window + non-aliasing; `event`
        //   points at the live event value pinned on the `trigger` stack frame
        //   for the whole walk (read-only `*const u8`).
        unsafe {
            runner(view, ctx, event);
        }
        i += 1;
    }
}
