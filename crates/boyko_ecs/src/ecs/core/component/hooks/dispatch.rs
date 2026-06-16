//! `trigger_on_*` dispatch fns — the cold, never-inlined entry points that
//! actually fire component lifecycle hooks (Phase 14a, plan §4.1 / §8 O2).
//!
//! Each of the four hook kinds gets one `#[cold] #[inline(never)]` fn, emitted
//! by the [`define_trigger!`] macro (the Phase 10 `set_table_*` macro-collapse
//! technique). The cheap per-archetype `ArchetypeFlags` bit-test is the
//! *caller's* gate (the no-hook hot path never reaches here — it is a single
//! `u16` load + `test`/`jz`); a `trigger_on_*` fn is entered ONLY when the
//! archetype proved that SOME component declares the corresponding hook, so
//! the `HOOKS[id]` read inside confirms whether THIS component does.
//!
//! Every fn mints a [`DeferredEcsMaster`] from the `world` pointer the
//! outermost apply holds and invokes the user `HookFn`. The mint is sound
//! because the call site dropped every `world`-derived `&mut Archetype` /
//! `&mut ComponentPool` before passing `world` here (plan §3 per-site
//! liveness; SAFETY-1), and hooks fire only inside the single-threaded apply
//! window (SAFETY-4).

use std::ptr::NonNull;

use crate::ecs::core::component::component_registry;
use crate::ecs::core::component::hooks::HookContext;
use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

/// Emits one `#[cold] #[inline(never)]` `trigger_on_*` fn selecting the named
/// `ComponentHooks` field. `$bit` names the `ArchetypeFlags` gate bit whose
/// caller-side test guards this fn (documentation only — the gate lives at the
/// call site, so the bit constant is not referenced in the body).
macro_rules! define_trigger {
    ($(#[$meta:meta])* $name:ident, $field:ident, $bit:literal) => {
        $(#[$meta])*
        ///
        /// Cold: only called when the archetype's gate bit
        #[doc = $bit]
        /// is set (the caller's `ArchetypeFlags` test). Reads `HOOKS[id]` once
        /// to confirm THIS component declares the hook, then fires it.
        #[cold]
        #[inline(never)]
        pub(crate) fn $name(
            world: NonNull<EcsMaster>,
            component_id: ComponentId,
            entity: Entity,
        ) {
            // The `HOOKS[id]` read is cold — the archetype already proved SOME
            // component is hooked; this confirms it is THIS one. One acquire
            // load + branch (mirrors `ComponentLayout::drop_fn`).
            if let Some(hooks) = component_registry::get_hooks(component_id.0)
                && let Some(f) = hooks.$field
            {
                // SAFETY (SAFETY-1 / SAFETY-4): `world` was minted via
                //   `NonNull::from(&mut *world)` at the call site AFTER every
                //   `world`-derived `&mut Archetype` / `&mut ComponentPool` was
                //   dropped (plan §3 per-site liveness), so the view aliases no
                //   live reborrow under Tree Borrows. Firing happens only in the
                //   single-threaded apply window (no concurrent reader), and the
                //   read-only view withholds every structural + `&mut`-into-pool
                //   method (Q-A2), so the hook cannot construct an aliasing
                //   `&mut` into a pool buffer.
                let view = unsafe { DeferredEcsMaster::from_world(world) };
                let ctx = HookContext { entity, component_id };
                // SAFETY (HookFn contract): the apply-window + non-aliasing
                //   invariants above are exactly what the `unsafe fn` HookFn
                //   requires of its caller.
                unsafe {
                    f(view, ctx);
                }
            }
        }
    };
}

define_trigger! {
    /// Fires `on_add` for `component_id` on `entity` (the component became
    /// newly present on the entity).
    trigger_on_add, on_add, "`ArchetypeFlags::ON_ADD_HOOK`"
}
define_trigger! {
    /// Fires `on_insert` for `component_id` on `entity` (the component was
    /// inserted — newly or via a bundle insert).
    trigger_on_insert, on_insert, "`ArchetypeFlags::ON_INSERT_HOOK`"
}
define_trigger! {
    /// Fires `on_replace` for `component_id` on `entity` (an existing value is
    /// about to be overwritten or the component is about to leave; the view
    /// still reads the OLD/dying value).
    trigger_on_replace, on_replace, "`ArchetypeFlags::ON_REPLACE_HOOK`"
}
define_trigger! {
    /// Fires `on_remove` for `component_id` on `entity` (the component is about
    /// to be removed; the view still reads the dying value).
    trigger_on_remove, on_remove, "`ArchetypeFlags::ON_REMOVE_HOOK`"
}
define_trigger! {
    /// Fires `on_despawn` for `component_id` on `entity` (the entity is being
    /// despawned; the view still reads the fully-intact dying row — Feature 2,
    /// Despawn-first ordering, before the per-component `on_replace`/`on_remove`
    /// passes).
    trigger_on_despawn, on_despawn, "`ArchetypeFlags::ON_DESPAWN_HOOK`"
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::component::hooks::{ComponentHooks, HookFn};
    use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

    const SEQ: Ordering = Ordering::SeqCst;

    // Top-of-space ids, disjoint from every integration test (≤ 619) and the
    // unit-test register_layout slots (≤ 465) + archetype_flags tests (508-511).
    const DISPATCH_HOOKED_ID: usize = 506;
    const DISPATCH_BARE_ID: usize = 507;

    // Per-test counters — NEVER shared. Lib `#[cfg(test)]` tests in one module
    // run on parallel threads; a single shared `static FIRES` would let the
    // two tests race each other's reset/read (a flaky-by-construction bug).
    static FIRES_INSTALLED: AtomicUsize = AtomicUsize::new(0);

    unsafe fn count_fire_installed(
        _w: DeferredEcsMaster<'_>,
        _c: crate::ecs::core::component::hooks::HookContext,
    ) {
        FIRES_INSTALLED.fetch_add(1, SEQ);
    }

    /// `trigger_on_add` fires the installed `on_add` exactly once. The trigger
    /// reads `HOOKS[id]`, confirms the field is `Some`, mints the view from the
    /// world pointer, and invokes the hook (the Wave-4 dispatch contract).
    #[test]
    fn trigger_on_add_fires_installed_hook_once() {
        let hooks = ComponentHooks {
            on_add: Some(count_fire_installed as HookFn),
            ..ComponentHooks::default()
        };
        // Write-once slot; first install in this binary must succeed.
        assert!(component_registry::try_set_hooks(DISPATCH_HOOKED_ID, hooks));

        let mut world = EcsMaster::new();
        let world_ptr = NonNull::from(&mut world);
        let entity = Entity::new(crate::ecs::identifiers::primitives::EntityId(0), 0);

        let before = FIRES_INSTALLED.load(SEQ);
        trigger_on_add(world_ptr, ComponentId(DISPATCH_HOOKED_ID), entity);
        assert_eq!(
            FIRES_INSTALLED.load(SEQ),
            before + 1,
            "trigger_on_add fires the installed on_add once"
        );

        // A kind the slot did NOT declare (on_remove) must NOT fire.
        trigger_on_remove(world_ptr, ComponentId(DISPATCH_HOOKED_ID), entity);
        assert_eq!(
            FIRES_INSTALLED.load(SEQ),
            before + 1,
            "trigger_on_remove must NOT fire when the slot's on_remove field is None"
        );
    }

    /// A component with NO entry in the `HOOKS` table is a no-op for every
    /// `trigger_on_*` (the `get_hooks` returns `None`). This is the safety net
    /// behind the per-archetype flag gate: even a wrongly-raised bit only
    /// reaches a `None` slot here.
    ///
    /// Uses a DEDICATED counter incremented by an installed hook on a SEPARATE
    /// id, so the assertion is "the dispatch for the BARE id did not touch this
    /// counter" — no shared static with the test above (no parallel race).
    static FIRES_BARE_GUARD: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn trigger_on_a_bare_component_is_a_noop() {
        let mut world = EcsMaster::new();
        let world_ptr = NonNull::from(&mut world);
        let entity = Entity::new(crate::ecs::identifiers::primitives::EntityId(0), 0);

        // DISPATCH_BARE_ID is never installed into HOOKS. Every trigger must be
        // a no-op (the `get_hooks` returns `None`). This counter is exclusive to
        // this test and only a hook would touch it — so it must stay 0.
        let before = FIRES_BARE_GUARD.load(SEQ);
        trigger_on_add(world_ptr, ComponentId(DISPATCH_BARE_ID), entity);
        trigger_on_insert(world_ptr, ComponentId(DISPATCH_BARE_ID), entity);
        trigger_on_replace(world_ptr, ComponentId(DISPATCH_BARE_ID), entity);
        trigger_on_remove(world_ptr, ComponentId(DISPATCH_BARE_ID), entity);
        assert_eq!(
            FIRES_BARE_GUARD.load(SEQ),
            before,
            "no hook installed at the bare id ⇒ every trigger is a no-op"
        );
    }
}
