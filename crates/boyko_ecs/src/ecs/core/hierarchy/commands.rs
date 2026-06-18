//! Hierarchy lifecycle hooks + the deferred commands that maintain
//! [`Children`] (Phase 19, CORE).
//!
//! # Two halves
//!
//! * **Hooks** (`child_of_on_insert` / `child_of_on_replace` /
//!   `children_on_replace`) fire synchronously at structural-op sites with the
//!   read-only [`DeferredEcsMaster`] view. They copy scalars out and enqueue
//!   deferred commands — they never mutate storage directly.
//! * **Command applies** (`LinkChildCommand` / `UnlinkChildCommand` /
//!   `ClearChildrenCommand`) run later under `&mut EcsMaster` at the apply
//!   window and perform the actual `Children` mutation / `ChildOf` removal.
//!
//! # OBS-FIRE-LOOP / F2 discipline (Tree-Borrows soundness)
//!
//! Every hook body follows the `fire_*_observers` discipline
//! (`observers/dispatch.rs`): a `world`-derived `&` (e.g. the `&ChildOf` /
//! `&Children` from [`DeferredEcsMaster::get_component`]) must NOT be live across
//! a [`DeferredEcsMaster::commands`] mint or a `despawn()` / enqueue call.
//! Scalars are copied out and the borrow dropped FIRST; only then are commands
//! minted. The view's `NonNull<EcsMaster>` reborrows the same world the borrow
//! came from, so a held `&` spanning the mint is the exact protected-tag
//! conflict that produced UB in Phase 14a (the F2 lesson). Miri under
//! `-Zmiri-tree-borrows` is the soundness oracle here, not code review.

use std::mem::MaybeUninit;

use crate::ecs::core::commands::command::Command;
use crate::ecs::core::commands::migration_helpers::{merged_archetype_id, migrate_entity_insert};
use crate::ecs::core::commands::remove_command::RemoveCommand;
use crate::ecs::core::component::hooks::HookContext;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::hierarchy::{CASCADE_FANOUT_INLINE, ChildOf, Children};

// ===========================================================================
// Deferred commands (W2)
// ===========================================================================

/// Deferred "link `child` into `parent`'s [`Children`]" command (Phase 19 W2).
///
/// Enqueued by [`child_of_on_insert`] after the new `ChildOf` is written;
/// applied under `&mut EcsMaster` at the apply window.
#[repr(C)]
pub(crate) struct LinkChildCommand {
    pub(crate) parent: Entity,
    pub(crate) child: Entity,
}

/// Deferred "unlink `child` from `parent`'s [`Children`]" command (Phase 19 W2).
///
/// Enqueued by [`child_of_on_replace`] reading the OLD (dying) parent;
/// a no-op if the link is not present (the apply guards both cases).
#[repr(C)]
pub(crate) struct UnlinkChildCommand {
    pub(crate) parent: Entity,
    pub(crate) child: Entity,
}

/// Deferred "remove `ChildOf` from every current child of `parent`" command
/// (Phase 19 W2). Backs `clear_children` (not `remove_children`, which removes
/// `ChildOf` from its listed children directly). Each removal fires
/// [`ChildOf`]'s `on_replace`, which enqueues an [`UnlinkChildCommand`].
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
//   (two `usize`-ish words, no borrowed references), so moving them across
//   threads is sound. The explicit impls document the intent for the
//   `Command: Send + 'static` queue bound.
unsafe impl Send for LinkChildCommand {}
unsafe impl Sync for LinkChildCommand {}
unsafe impl Send for UnlinkChildCommand {}
unsafe impl Sync for UnlinkChildCommand {}
unsafe impl Send for ClearChildrenCommand {}
unsafe impl Sync for ClearChildrenCommand {}
unsafe impl Send for DespawnWithoutChildrenCommand {}
unsafe impl Sync for DespawnWithoutChildrenCommand {}

impl Command for LinkChildCommand {
    fn apply(self, world: &mut EcsMaster) {
        let parent = self.parent;
        let child = self.child;

        // Dangling-parent guard: the parent may have been despawned between the
        // hook firing and this apply. A no-op keeps the invariant rather than
        // resurrecting a dead collection.
        if !world.has_entity(parent) {
            return;
        }

        match world.get_component_mut::<Children>(parent) {
            // Parent already has a `Children` — pure in-place push (no archetype
            // change). `DerefMut` stamps the changed tick; harmless for a
            // structural relationship op.
            Some(mut children) => children.push(child),
            None => {
                // First child: route the insert through the audited migration
                // machinery — `Children` is a `Bundle` itself (Phase 22 D7,
                // `impl_self_bundle!`; the Phase-19 newtype is deleted). This
                // fires `on_add` + `on_insert` only — `Children` registers
                // neither, so no spurious cascade.
                //
                // `has_entity(parent)` above proved the slot is non-null and
                // generation-matched; the sequential exclusive borrows
                // (`has_entity` → `get_component_mut` → `entities_inland[..]`)
                // hold nothing live across the migrate.
                let inland = world.entity_master.entities_inland[parent.id().0];
                // SAFETY (verbatim copy of the audited `insert_command.rs:74`
                //   F1 pattern): `archetype_ptr` is write-capable, stable,
                //   interior-mutable (`SharedReadWrite`, F4-rooted) slab
                //   provenance — it survives sibling structural writes under
                //   TB/SB (the whole slab element is `UnsafeCell`-wrapped).
                //   Non-null + generation-matched by the preceding `has_entity`,
                //   so the slot is live.
                let src = unsafe { (*inland.archetype_ptr()).id() };
                let tgt = merged_archetype_id::<Children>(world, src);
                migrate_entity_insert::<Children>(
                    world,
                    parent,
                    src,
                    tgt,
                    Children::with_one(child),
                );
            }
        }
    }
}

impl Command for UnlinkChildCommand {
    fn apply(self, world: &mut EcsMaster) {
        // No remove-on-empty (R2 W1): an emptied `Children` is retained to avoid
        // archetype thrash on `0↔1↔0` oscillation. A missing `Children` or an
        // absent child are both harmless no-ops (the spurious-unlink path from
        // the self-ref / dangling guards lands here).
        let Some(mut children) = world.get_component_mut::<Children>(self.parent) else {
            return;
        };
        children.swap_remove_entity(self.child);
    }
}

impl Command for ClearChildrenCommand {
    fn apply(self, world: &mut EcsMaster) {
        // #17883-safe: re-read the CURRENT children per turn, copy one child out,
        // drop the borrow, THEN enqueue. Each child's `ChildOf` removal is
        // DEFERRED (pushed to the queue, applied on a later drain turn), so the
        // `Children` collection does NOT shrink during this loop — iterating by
        // index `0..len` visits each child exactly once. Routing through
        // `RemoveCommand<ChildOf>` fires `ChildOf::on_replace` → the audited
        // unlink path, so the collection empties consistently.
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
// ChildOf hooks (W3)
// ===========================================================================

/// Cold, non-allocating warning helper for the self-ref / dangling guards
/// (Phase 19 M1). The message is a `&'static str` literal — no `format!`, no
/// heap. Release builds pay nothing (the body is `#[cfg(debug_assertions)]`);
/// debug builds surface the warning via `eprintln!`, the ECS crate's accepted
/// no-dependency warn channel (precedent: `schedule_builder.rs:1287`). `reason`
/// is the entire payload — no entity-id context is formatted in.
#[cold]
fn warn_invalid_child_of(reason: &'static str) {
    #[cfg(debug_assertions)]
    eprintln!("boyko-ecs: hierarchy: {reason}");
    #[cfg(not(debug_assertions))]
    let _ = reason;
}

/// `ChildOf::on_insert` (LINK, Phase 19 §3 / W3).
///
/// Fires after the NEW `ChildOf` is written (fresh add or reparent insert).
/// Guards self-reference and a dangling parent reactively (remove the bad
/// `ChildOf`, touch no collection); otherwise enqueues a [`LinkChildCommand`].
///
/// # Safety
///
/// `HookFn` contract (`hooks/dispatch.rs`): invoked only inside the
/// single-threaded apply window with a view that withholds every structural +
/// `&mut`-into-pool method. The body holds no `world`-derived `&` across the
/// `commands()` mint (F2).
pub(crate) unsafe fn child_of_on_insert(mut view: DeferredEcsMaster<'_>, ctx: HookContext) {
    let child = ctx.entity;
    // Copy the parent out; the `&ChildOf` borrow ends at the `?`/`;`.
    let Some(&ChildOf(parent)) = view.get_component::<ChildOf>(child) else {
        return;
    };

    // Self-reference: reject reactively. The removal fires `on_replace` →
    // enqueues a spurious `UnlinkChild(child, child)` that no-ops at apply.
    if parent == child {
        warn_invalid_child_of("ChildOf points at itself; removing the self-referential link");
        view.commands().entity(child).remove::<ChildOf>();
        return;
    }

    // Dangling parent: the prospective parent does not exist. Reject reactively.
    if !view.has_parent(parent) {
        warn_invalid_child_of("ChildOf points at a non-existent parent; removing the dangling link");
        view.commands().entity(child).remove::<ChildOf>();
        return;
    }

    view.commands().add(LinkChildCommand { parent, child });
}

/// `ChildOf::on_replace` (UNLINK, Phase 19 §3 / W3).
///
/// Fires before the OLD `ChildOf` leaves (reparent overwrite, explicit remove,
/// or despawn). Reads the dying value and enqueues an [`UnlinkChildCommand`] for
/// the OLD parent. Harmless if the link is absent (the apply guards it).
///
/// # Safety
///
/// See [`child_of_on_insert`].
pub(crate) unsafe fn child_of_on_replace(mut view: DeferredEcsMaster<'_>, ctx: HookContext) {
    let child = ctx.entity;
    // The OLD/dying value (the remove-migration fires this before the value
    // leaves the source row). Copy the parent out; the `&` ends here.
    let Some(&ChildOf(old_parent)) = view.get_component::<ChildOf>(child) else {
        return;
    };
    view.commands().add(UnlinkChildCommand { parent: old_parent, child });
}

// ===========================================================================
// Children cascade hook (W4)
// ===========================================================================

/// `Children::on_replace` (CASCADE despawn, Phase 19 §3 / W4).
///
/// Fired before `Children` leaves the parent (in `delete_entity`, the
/// #20106-correct pre-remove order) reading the CURRENT collection. Enqueues a
/// recursive `despawn` per child; recursion emerges through the deferred queue
/// (each inner `delete_entity` at depth ≥ 1 no-ops its own drain — the single
/// outermost drain absorbs grandchildren).
///
/// Suppressed when the [`super::commands`]-internal `CASCADE_SUPPRESS`
/// thread-local is set (the `despawn_without_children` opt-out).
///
/// # Safety
///
/// See [`child_of_on_insert`]. The two paths differ in how they avoid holding a
/// `world`-derived `&` across `commands()` (inline = copy-to-stack-then-drop;
/// wide = re-derive per turn).
pub(crate) unsafe fn children_on_replace(mut view: DeferredEcsMaster<'_>, ctx: HookContext) {
    // Opt-out: `despawn_without_children` sets this for exactly one
    // `delete_entity`, so the children survive (with a now-dangling `ChildOf`).
    if cascade_suppressed() {
        return;
    }

    let parent = ctx.entity;

    // Read the count first; the `&Children` ends at the block close.
    let n = {
        let Some(children) = view.get_component::<Children>(parent) else {
            return;
        };
        children.len()
        // <-- `&Children` drops here.
    };
    if n == 0 {
        return;
    }

    if n <= CASCADE_FANOUT_INLINE {
        // Inline fast path: copy the children into a stack buffer, drop the
        // `&Children`, THEN mint `commands()` and enqueue.
        let mut buf: [MaybeUninit<Entity>; CASCADE_FANOUT_INLINE] =
            [const { MaybeUninit::uninit() }; CASCADE_FANOUT_INLINE];
        {
            let Some(children) = view.get_component::<Children>(parent) else {
                return;
            };
            let slice = children.as_slice();
            // `n` was read from the same collection one statement ago under the
            // single-threaded apply window; nothing can have mutated it.
            debug_assert!(n <= slice.len(), "cascade: children shrank between reads");
            for (i, slot) in buf[..n].iter_mut().enumerate() {
                slot.write(slice[i]);
            }
            // <-- `&Children` drops here, BEFORE `commands()` is minted (F2).
        }
        let mut cmds = view.commands();
        for slot in buf[..n].iter() {
            // SAFETY (M2 — the ONE new `unsafe` in Phase 19): every `buf[i]` for
            //   `i < n` was written by the immediately-preceding loop from the
            //   valid length-`n` slice; only `buf[..n]` is read here. `Entity` is
            //   `Copy`, so `assume_init` reads a bitwise-valid value and leaves
            //   nothing to drop.
            let child = unsafe { slot.assume_init() };
            cmds.entity(child).despawn();
        }
    } else {
        // Wide cold path: re-derive the `&Children` per turn (OBS-FIRE-LOOP
        // taken to the extreme) — no buffer, no `unsafe`. The `&` must not span
        // the `commands()` / `despawn()` call.
        let mut i = 0usize;
        loop {
            let next = {
                let Some(children) = view.get_component::<Children>(parent) else {
                    return;
                };
                if i >= children.len() {
                    return;
                }
                children.as_slice()[i]
                // <-- `&Children` drops here.
            };
            view.commands().entity(next).despawn();
            i += 1;
        }
    }
}

// ===========================================================================
// Cascade-suppress thread-local (W4)
// ===========================================================================

use std::cell::Cell;

thread_local! {
    /// When set, [`children_on_replace`] returns immediately — the
    /// `despawn_without_children` opt-out. Thread-local for the same reason as
    /// `HOOK_DRAIN_DEPTH` (`hooks/scope.rs`): all hook firing runs on the
    /// single-threaded apply window, and a per-thread cell cannot be frozen by
    /// any `&mut EcsMaster` reborrow (the F2 invariant).
    static CASCADE_SUPPRESS: Cell<bool> = const { Cell::new(false) };
}

/// Reads the cascade-suppress flag for the current thread.
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
