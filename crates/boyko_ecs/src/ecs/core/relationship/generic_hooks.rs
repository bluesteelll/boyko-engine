//! Generic, monomorphized relationship-maintenance hook bodies (Relations v1).
//!
//! The three bodies below are the SAME hooks Phase-19 wrote by hand for
//! `ChildOf` / `Children`, generalized over `R: Relationship` / `T:
//! RelationshipTarget`. Each monomorphizes to one concrete
//! `unsafe fn(DeferredEcsMaster, HookContext)` per relation type — a bare fn
//! pointer in the `HOOKS` table, identical machine code to the hand-written
//! `child_of_on_insert`. No `dyn`, no vtable, no `TypeId` dispatch.
//!
//! # OBS-FIRE-LOOP / F2 discipline (Tree-Borrows soundness)
//!
//! Every body follows the `fire_*_observers` discipline: a `world`-derived `&`
//! (the `&R` / `&T` from [`DeferredEcsMaster::get_component`]) must NOT be live
//! across a [`DeferredEcsMaster::commands`] mint or a `despawn()` / enqueue call.
//! Scalars are copied out and the borrow dropped FIRST; only then are commands
//! minted. Miri under `-Zmiri-tree-borrows` is the soundness oracle here.
//!
//! # Enqueue-only contract (BUG-P19-TB-1)
//!
//! The hooks NEVER mutate storage inline — they only enqueue into
//! `deferred_hook_queue`. The `apply_via_raw_twin` disjoint-allocation drain (the
//! Phase-19 fix) depends on it. This holds STRUCTURALLY: a hook's
//! [`DeferredEcsMaster`] exposes no `&mut`-into-storage method, so the
//! `*_risky` collection mutators are unreachable from any hook body (W2).

use std::mem::MaybeUninit;

use crate::ecs::core::component::hooks::HookContext;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::CASCADE_FANOUT_INLINE;
use crate::ecs::core::hierarchy::commands::cascade_suppressed;
use crate::ecs::core::relationship::{LinkCommand, Relationship, RelationshipSourceCollection, RelationshipTarget, UnlinkCommand, relationship_link_suppressed};

// ===========================================================================
// W4 / C1 — cyclic-cascade termination (cross-level, NOT a per-hook guard)
// ===========================================================================
//
// A `LINKED_DESPAWN` cascade recurses through the FLAT deferred-hook queue (the
// `drain_deferred_hook_queue` `while !is_empty()` loop), not the call stack: each
// cascade level enqueues despawns that surface as the NEXT drain turn, every
// level firing this hook at `hook_drain_depth == 1`. A per-hook depth guard here
// could therefore never accumulate across levels (the original W4 RAII guard
// always observed depth 1). Cyclic graphs (A→B→…→A) terminate NATURALLY: a
// re-entered despawn of an already-freed entity is a generation-checked no-op in
// `delete_entity_core`, so each cyclic entity is despawned exactly once and the
// live set strictly shrinks per real despawn. The only remaining guard against a
// *pathological* non-terminating re-enqueue is the cross-level
// `MAX_HOOK_DRAIN_TURNS` backstop in `drain_deferred_hook_queue` — the right
// layer, since the recursion lives in that flat queue. This body therefore holds
// NO depth guard.

// ===========================================================================
// Generic hook bodies
// ===========================================================================

/// Generic LINK hook (`<R>::on_insert`). Fires after the NEW `R` is written
/// (fresh add or re-target insert). Guards self-reference and a dangling target
/// reactively (remove the bad `R`, touch no collection); otherwise enqueues a
/// [`LinkCommand<R>`]. The byte-for-byte generalization of Phase-19
/// `child_of_on_insert`.
///
/// # Safety
///
/// `HookFn` contract: invoked only inside the single-threaded apply window with a
/// view that withholds every structural + `&mut`-into-storage method. The body
/// holds no `world`-derived `&` across the `commands()` mint (F2).
pub unsafe fn relationship_on_insert<R: Relationship>(
    mut view: DeferredEcsMaster<'_>,
    ctx: HookContext,
) {
    let source = ctx.entity;
    // Copy the target out; the `&R` borrow ends at the `else`/`;`.
    let target = match view.get_component::<R>(source) {
        Some(r) => r.target(),
        None => return,
    };

    // Self-reference: reject reactively unless the relation opts in. The removal
    // fires `on_replace` → enqueues a spurious unlink that no-ops at apply.
    if target == source && !R::ALLOW_SELF_REFERENTIAL {
        view.commands().entity(source).remove::<R>();
        return;
    }

    // Dangling-target guard: the prospective target must exist (B3 — the renamed
    // liveness method).
    if !view.is_alive(target) {
        view.commands().entity(source).remove::<R>();
        return;
    }

    // 1:1 eviction (Collection == `Exclusive`) is detected at APPLY, NOT here
    // (Relations v1.1). This hook body is UNCHANGED for 1:1 — it only ENQUEUES the
    // `LinkCommand` below; `LinkCommand::apply` reads
    // `source_to_evict_before_add()` on the reverse collection and, on an occupied
    // `Exclusive` slot held by a distinct incumbent, overwrites the slot, fires
    // `OnUnlink{incumbent}` once, and defers a `RemoveCommand` to clear the
    // incumbent's dangling FK. The `Vec` one-to-many collection returns the trait
    // default `None` there, so that whole eviction branch const-folds away
    // (byte-identical v1 apply). Keeping detection at apply time avoids a new
    // re-entrant surface in this enqueue-only hook.

    // BUG-EDGE-CLONE-1: during a deep clone the FK is a VERBATIM copy still pointing
    // at the ORIGINAL (un-remapped) target — enqueuing a link here would leak the
    // clone into the SOURCE subtree's reverse collection. The clone's relink pass
    // (`relationship_clone_relink`) is the sole linker, establishing exactly one link
    // toward the remapped clone target after the FK is remapped. Suppress the stale
    // link (the reactive self-ref / dangling guards above still ran). Relation-
    // agnostic: every relation (incl. `ChildOf`) routes through this body.
    if relationship_link_suppressed() {
        return;
    }

    view.commands().add(LinkCommand::<R> {
        target,
        source,
        _marker: core::marker::PhantomData,
    });
}

/// Generic UNLINK hook (`<R>::on_replace`). Fires before the OLD `R` leaves
/// (re-target overwrite, explicit remove, or despawn). Reads the dying value and
/// enqueues an [`UnlinkCommand<R>`] for the OLD target. Harmless if the link is
/// absent (the apply guards it). The generalization of Phase-19
/// `child_of_on_replace`.
///
/// # Safety
///
/// See [`relationship_on_insert`].
pub unsafe fn relationship_on_replace<R: Relationship>(
    mut view: DeferredEcsMaster<'_>,
    ctx: HookContext,
) {
    let source = ctx.entity;
    // The OLD/dying value (the remove-migration fires this before the value
    // leaves the source row). Copy the target out; the `&` ends here.
    let old_target = match view.get_component::<R>(source) {
        Some(r) => r.target(),
        None => return,
    };
    view.commands().add(UnlinkCommand::<R> {
        target: old_target,
        source,
        _marker: core::marker::PhantomData,
    });
    // v1: `R::Target::RETAIN_EMPTY` is ALWAYS `true` (W1) — `UnlinkCommand::apply`
    // leaves an emptied collection in place (no migration). The `RETAIN_EMPTY ==
    // false` branch (queue removal of the now-useless `R::Target`, the Bevy
    // default) is RESERVED but UNIMPLEMENTED in v1 (a new re-entrant edge, W1/O3).
}

/// Generic CASCADE hook (`<T>::on_replace`). Fired before `T` leaves the target
/// (in `delete_entity`, the pre-remove order) reading the CURRENT collection.
///
/// When `T::LINKED_DESPAWN`, enqueues a recursive `despawn` per source (the
/// Phase-19 `Children` cascade, generalized); recursion emerges through the
/// deferred queue. When NOT `LINKED_DESPAWN`, only unlinks the sources' `R`
/// (no recursive despawn). The body const-folds to one branch per relation type.
///
/// # Safety
///
/// See [`relationship_on_insert`]. The inline and wide paths differ in how they
/// avoid holding a `world`-derived `&` across `commands()` (inline =
/// copy-to-stack-then-drop; wide = re-derive per turn).
pub unsafe fn relationship_target_on_replace<T: RelationshipTarget>(
    mut view: DeferredEcsMaster<'_>,
    ctx: HookContext,
) {
    let target = ctx.entity;

    if !T::LINKED_DESPAWN {
        // Non-cascading target: unlink every source's `R` (enqueue a remove of
        // the source-of-truth component, which fires `R::on_replace` → the
        // audited unlink). No recursive despawn. Re-derive the `&T` per turn so
        // no `world`-derived `&` spans the `commands()` mint (OBS-FIRE-LOOP).
        let mut i = 0usize;
        loop {
            let next = {
                let Some(reverse) = view.get_component::<T>(target) else {
                    return;
                };
                // O(1) indexed read (the Phase-19 `as_slice()[i]` access). A
                // `None` (out of bounds) ends the walk.
                let Some(next) = reverse.collection().get(i) else {
                    return;
                };
                next
                // <-- `&T` drops here.
            };
            view.commands().entity(next).remove::<T::Source>();
            i += 1;
        }
    }

    // LINKED_DESPAWN cascade from here on.

    // Opt-out: `despawn_without_children` (and any future relation-agnostic
    // suppress) sets this for exactly one `delete_entity`, so the sources survive
    // (with a now-dangling `R`).
    if cascade_suppressed() {
        return;
    }

    // W4 / C1: no per-hook depth guard here — a `LINKED_DESPAWN` cascade recurses
    // through the flat drain queue (every level fires at `hook_drain_depth == 1`),
    // so a per-hook guard can never accumulate. Cyclic graphs terminate naturally
    // via `delete_entity_core`'s already-dead-no-op; the cross-level
    // `MAX_HOOK_DRAIN_TURNS` backstop in `drain_deferred_hook_queue` bounds a
    // pathological runaway. See the module-level W4 / C1 note above.

    // Read the count first; the `&T` ends at the block close.
    let n = {
        let Some(reverse) = view.get_component::<T>(target) else {
            return;
        };
        reverse.collection().len()
        // <-- `&T` drops here.
    };
    if n == 0 {
        return;
    }

    if n <= CASCADE_FANOUT_INLINE {
        // Inline fast path: copy the sources into a stack buffer, drop the `&T`,
        // THEN mint `commands()` and enqueue.
        let mut buf: [MaybeUninit<Entity>; CASCADE_FANOUT_INLINE] =
            [const { MaybeUninit::uninit() }; CASCADE_FANOUT_INLINE];
        {
            let Some(reverse) = view.get_component::<T>(target) else {
                return;
            };
            // `n` was read from the same collection one statement ago under the
            // single-threaded apply window; nothing can have mutated it.
            debug_assert!(
                n <= reverse.collection().len(),
                "cascade: sources shrank between reads"
            );
            for (slot, source) in buf[..n].iter_mut().zip(reverse.collection().iter()) {
                slot.write(source);
            }
            // <-- `&T` drops here, BEFORE `commands()` is minted (F2).
        }
        let mut cmds = view.commands();
        for slot in buf[..n].iter() {
            // SAFETY (U1 — the relocated Phase-19 M2 unsafe): every `buf[i]` for
            //   `i < n` was written by the immediately-preceding loop from the
            //   valid length-`n` collection (the `zip` stops at `n`); only
            //   `buf[..n]` is read here. `Entity` is `Copy`, so `assume_init`
            //   reads a bitwise-valid value and leaves nothing to drop.
            let source = unsafe { slot.assume_init() };
            cmds.entity(source).despawn();
        }
    } else {
        // Wide cold path: re-derive the `&T` per turn (OBS-FIRE-LOOP taken to the
        // extreme) — no buffer, no `unsafe`. The `&` must not span the
        // `commands()` / `despawn()` call.
        let mut i = 0usize;
        loop {
            let next = {
                let Some(reverse) = view.get_component::<T>(target) else {
                    return;
                };
                // O(1) indexed read (the Phase-19 `as_slice()[i]` access). A
                // `None` (out of bounds) ends the walk.
                let Some(next) = reverse.collection().get(i) else {
                    return;
                };
                next
                // <-- `&T` drops here.
            };
            view.commands().entity(next).despawn();
            i += 1;
        }
    }
}
