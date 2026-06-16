//! `fire_*_observers` dispatch fns — the cold, never-inlined entry points that
//! fire component lifecycle observers (Phase 14b §5 / R3 §2).
//!
//! Sibling of [`hooks::dispatch`](crate::ecs::core::component::hooks::dispatch):
//! where `trigger_on_*` reads the process-global write-once `HOOKS` table, a
//! `fire_*_observers` fn reads the per-world, runtime-mutable
//! [`ObserverRegistry`](crate::ecs::core::component::observers::ObserverRegistry)
//! that lives inside the world's `ArchetypeMaster`, and fires *every* observer
//! registered for `(kind, component_id)` (a list, not a single fn-ptr).
//!
//! Each kind gets one `#[cold] #[inline(never)]` fn, emitted by the
//! [`define_fire_observers!`] macro. The cheap per-archetype `ArchetypeFlags`
//! bit-test is the *caller's* gate (the structural-op fire site) — the
//! no-observer hot path never reaches here. A `fire_*_observers` fn is entered
//! ONLY when the archetype proved that SOME component carries the matching
//! `ON_*_OBSERVER` bit, so the list read inside confirms whether THIS component
//! does.
//!
//! # OBS-FIRE-LOOP invariant (the single most dangerous spot in Phase 14b)
//!
//! No registry `&` — nor any `world`-derived `&` (`world.as_ref()`,
//! `archetype_master()`, the `&Vec<ObserverEntry>`, or any sub-borrow of them)
//! — may be live across the [`DeferredEcsMaster::from_world`] mint or the
//! `(entry.runner)(view, …)` call. Each loop turn re-derives its borrow, copies
//! one [`ObserverEntry`] by value, and lets every borrow end **before** the view
//! is minted. The registry holding the `Vec` lives *inside* the same world the
//! view reborrows, so a held registry `&` spanning the `from_world` reborrow is
//! the exact Tree-Borrows protected-tag conflict (the F2-class hazard) that
//! produced UB in Phase 14a. Re-reading `len()` each turn is cheap and correct:
//! the registry is provably immutable across this window (its only mutators,
//! `add_observer` / `remove_observer`, require `&mut EcsMaster`, which cannot be
//! live during a fire — see R2 §7). Miri under `-Zmiri-tree-borrows` is the
//! soundness oracle here, not code review.

use std::ptr::NonNull;

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::component::observers::{ObserverContext, ObserverEntry, ObserverKind};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Emits one `#[cold] #[inline(never)]` `fire_*_observers` fn that walks the
/// `(kind, component_id)` observer list and fires each runner.
///
/// `$kind_idx` is the dense `[kind]` index into `ObserverLists::by_kind_component`
/// (kept as a literal so the indexing is a constant). `$kind` is the
/// [`ObserverKind`] stamped into each fired [`ObserverContext`]. The body is the
/// robust per-iteration-re-derive loop (R3 §2) — see the OBS-FIRE-LOOP module
/// invariant: it holds NO registry `&` across the view mint or the runner call.
macro_rules! define_fire_observers {
    ($(#[$meta:meta])* $name:ident, $kind:expr, $kind_idx:literal) => {
        $(#[$meta])*
        ///
        /// Cold: invoked only when the archetype's `ON_*_OBSERVER` bit is set
        /// (the caller's `ArchetypeFlags` test). Fires every observer registered
        /// for `(kind, component_id)`, in registration order, copying each
        /// [`ObserverEntry`] out before minting the view (OBS-FIRE-LOOP).
        #[cold]
        #[inline(never)]
        pub(crate) fn $name(
            world: NonNull<EcsMaster>,
            component_id: ComponentId,
            entity: Entity,
        ) {
            let mut i = 0usize;
            loop {
                // Re-derive a FRESH, SHORT-LIVED registry `&` each turn; copy the
                // 16 B `ObserverEntry` out; drop the `&` at this block's close —
                // BEFORE the view is minted. Re-reading `len()` each turn is cheap
                // and correct (the registry is immutable across this window, R2 §7).
                let entry: ObserverEntry = {
                    // SAFETY (OBS-FIRE / OBS-2): `world` was minted via
                    //   `NonNull::from(&mut *self)` at the call site AFTER every
                    //   `world`-derived `&mut Archetype` / `&mut ComponentPool` was
                    //   dropped (per-site liveness, SAFETY-1), so the shared read
                    //   aliases no live reborrow. This `&` is re-derived per
                    //   iteration and dropped at this block boundary, BEFORE any
                    //   `world`-derived view is minted. Single-threaded apply
                    //   window; the registry is mutated only via `&mut self`, which
                    //   cannot be live here (R2 §7).
                    let reg = unsafe {
                        &world.as_ref().archetype_master().observer_registry
                    };
                    let Some(list) = reg.fire_list($kind, component_id) else {
                        break;
                    };
                    if i >= list.len() {
                        break;
                    }
                    list[i] // Copy out (`ObserverEntry`: 16 B POD) — the `&`s end HERE.
                };
                // No registry `&` (nor any `world`-derived `&`) is live past this point.
                // SAFETY (SAFETY-1 / SAFETY-4): `world` aliases no live reborrow at
                //   the mint (the registry `&` above is dead); firing happens only in
                //   the single-threaded apply window, and the read-only view withholds
                //   every structural + `&mut`-into-pool method (Q-A2).
                let view = unsafe { DeferredEcsMaster::from_world(world) };
                // SAFETY (OBS-FIRE-CALL): the `ObserverFn` contract == the `HookFn`
                //   contract — apply-window + non-aliasing (the view withholds
                //   structural + `&mut`-into-pool methods), exactly what the
                //   `unsafe fn` requires of its caller.
                unsafe {
                    (entry.runner)(
                        view,
                        ObserverContext { entity, component_id, kind: $kind },
                    );
                }
                i += 1;
            }
        }
    };
}

define_fire_observers! {
    /// Fires every `on_add` observer for `component_id` on `entity` (the
    /// component became newly present on the entity).
    fire_on_add_observers, ObserverKind::Add, 0
}
define_fire_observers! {
    /// Fires every `on_insert` observer for `component_id` on `entity` (the
    /// component was inserted — newly or via a bundle insert).
    fire_on_insert_observers, ObserverKind::Insert, 1
}
define_fire_observers! {
    /// Fires every `on_replace` observer for `component_id` on `entity` (an
    /// existing value is about to be overwritten, or the component is about to
    /// leave; the view still reads the OLD/dying value).
    fire_on_replace_observers, ObserverKind::Replace, 2
}
define_fire_observers! {
    /// Fires every `on_remove` observer for `component_id` on `entity` (the
    /// component is about to be removed; the view still reads the dying value).
    fire_on_remove_observers, ObserverKind::Remove, 3
}
define_fire_observers! {
    /// Fires every `on_despawn` observer for `component_id` on `entity` (the
    /// entity is being despawned; the view still reads the fully-intact dying
    /// row — Feature 2, Despawn-first ordering).
    fire_on_despawn_observers, ObserverKind::Despawn, 4
}
