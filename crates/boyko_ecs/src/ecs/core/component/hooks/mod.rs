//! Component lifecycle hooks (Phase 14a).
//!
//! This module hosts the additive infrastructure for `on_add` / `on_insert` /
//! `on_replace` / `on_remove` callbacks bound to component types. See
//! `docs/PHASE-14-OBSERVERS-PLAN-ROUND2.md`.
//!
//! `on_despawn` (an entity-level despawn hook, distinct from `on_remove`) is
//! deferred to Phase 14b: a no-fire stub on the 14a surface would let users
//! register a hook that never runs, so the kind is intentionally absent until
//! its dispatch site exists.
//!
//! Wave 4 adds the `dispatch.rs` `trigger_on_*` fns that actually fire hooks;
//! Waves 1-3 ship only the data structures, the per-archetype flag bitset, the
//! cold `HOOKS` table plumbing, the deferred queue, and the read-only view.

use crate::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::ComponentId;

pub mod archetype_flags;
pub mod builder;
pub mod deferred_master;
pub mod dispatch;
pub mod scope;

/// Type-erased lifecycle-hook function pointer.
///
/// Mirrors the `DropFn = unsafe fn(*mut u8)` precedent
/// (component_registry.rs:67) — a plain fn pointer, zero-alloc, monomorphised
/// at registration. `unsafe` because the dispatch site (always a
/// `trigger_on_*` fn, Wave 4) guarantees the apply-window-only + non-aliasing
/// invariants the body relies on (plan §5 SAFETY-1 / SAFETY-4).
pub type HookFn = unsafe fn(DeferredEcsMaster<'_>, HookContext);

/// Per-[`ComponentId`] lifecycle hooks. Stored in the parallel cold `HOOKS`
/// table (plan Q5), NOT inline in `ComponentLayout` (keeps the latter at 56 B
/// — TRIPWIRE 2).
///
/// `None` slots are zero-cost: "is this kind hooked?" is `Option::is_some()`,
/// the same pattern `ComponentLayout::drop_fn` uses. All-`None` (the `Default`)
/// is the value for any component without a `#[component(...)]` attribute or
/// runtime builder.
///
/// # Derive XOR runtime
///
/// A type populates its `HOOKS` slot via EITHER the `#[component(...)]` derive
/// OR the runtime [`register_component_hooks`] builder — never both. Each slot
/// is written exactly once (`OnceLock::set`); there is no merge between the two
/// mechanisms (the builder seeds from this `Default`, and registering hooks for
/// a derive-hooked type panics).
///
/// [`register_component_hooks`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::register_component_hooks
///
/// # Send + Sync (plan §8 O1)
///
/// Auto-derived `Send + Sync` holds with no `unsafe impl`: every field is a
/// plain `unsafe fn` pointer, and fn pointers are unconditionally `Send +
/// Sync`. This is what lets `static HOOKS: [OnceLock<ComponentHooks>; N]` exist
/// (it requires `ComponentHooks: Send + Sync`).
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct ComponentHooks {
    /// Fired after a component is newly added to an entity.
    pub on_add: Option<HookFn>,
    /// Fired after a component is inserted (newly or via bundle insert).
    pub on_insert: Option<HookFn>,
    /// Fired before an existing component value is overwritten.
    pub on_replace: Option<HookFn>,
    /// Fired before a component is removed (reads the dying value).
    pub on_remove: Option<HookFn>,
}

/// Context passed to every hook. Bevy's `MaybeLocation` /
/// `RelationshipHookMode` are omitted — boyko has neither subsystem
/// (research §1). `HookContext` is `{ entity, component_id }` only.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct HookContext {
    /// The entity the structural op targets.
    pub entity: Entity,
    /// Which component triggered the hook.
    pub component_id: ComponentId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::identifiers::primitives::EntityId;

    #[test]
    fn component_hooks_default_is_all_none() {
        let h = ComponentHooks::default();
        assert!(h.on_add.is_none(), "Default on_add is None (no-hook value)");
        assert!(h.on_insert.is_none(), "Default on_insert is None");
        assert!(h.on_replace.is_none(), "Default on_replace is None");
        assert!(h.on_remove.is_none(), "Default on_remove is None");
    }

    #[test]
    fn component_hooks_is_copy() {
        // The cold `HOOKS` table stores `OnceLock<ComponentHooks>`; `Copy` lets
        // `get_hooks` hand out `&'static` and lets the builder seed cheaply.
        fn assert_copy<T: Copy>() {}
        assert_copy::<ComponentHooks>();
    }

    #[test]
    fn hook_fn_pointer_fields_are_niche_optimized() {
        // Mirrors `ComponentLayout::drop_fn`: `Option<fn>` is 8 B (niche).
        assert_eq!(
            std::mem::size_of::<Option<HookFn>>(),
            std::mem::size_of::<HookFn>(),
            "Option<HookFn> must be niche-optimized to a bare fn-pointer width"
        );
    }

    #[test]
    fn hook_context_carries_entity_and_component_id() {
        let ctx = HookContext {
            entity: Entity::new(EntityId(3), 7),
            component_id: ComponentId(42),
        };
        assert_eq!(ctx.entity.id().0, 3, "HookContext exposes the target entity id");
        assert_eq!(ctx.entity.generation(), 7, "and its generation");
        assert_eq!(ctx.component_id.0, 42, "and the triggering component id");
    }

    /// `ComponentHooks` and `HookContext` are `Send + Sync` (fn-pointer-only /
    /// plain-data) — the property that lets `static HOOKS: [OnceLock<..>; N]`
    /// exist (plan §8 O1).
    #[test]
    fn hooks_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ComponentHooks>();
        assert_send_sync::<HookContext>();
    }
}
