//! Component lifecycle hooks (Phase 14a).
//!
//! This module hosts the additive infrastructure for `on_add` / `on_insert` /
//! `on_replace` / `on_remove` callbacks bound to component types. See
//! `docs/PHASE-14-OBSERVERS-PLAN-ROUND2.md`.
//!
//! `on_despawn` (an entity-level despawn hook, distinct from `on_remove`) was
//! deferred past Phase 14a and is now present (Feature 2): it fires once per
//! dying entity at the despawn site, BEFORE any component drops, gated by
//! [`ArchetypeFlags::ON_DESPAWN_HOOK`].
//!
//! [`ArchetypeFlags::ON_DESPAWN_HOOK`]: crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags::ON_DESPAWN_HOOK
//!
//! Wave 4 adds the `dispatch.rs` `trigger_on_*` fns that actually fire hooks;
//! Waves 1-3 ship only the data structures, the per-archetype flag bitset, the
//! cold `HOOKS` table plumbing, the deferred queue, and the read-only view.

use std::fmt;

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
    /// Fired once per dying entity at despawn, BEFORE any component drops
    /// (Feature 2 — the entity-level despawn hook the Phase-14a surface cut).
    /// Distinct from `on_remove`: `on_despawn` means "the whole entity is
    /// dying", and a handler reads the fully-intact entity (Despawn fires before
    /// the per-component `on_replace`/`on_remove` passes).
    pub on_despawn: Option<HookFn>,
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

/// Error returned by
/// [`register_hooks_by_id`](crate::ecs::core::component::component_registry::register_hooks_by_id)
/// (Phase 22 D8) — the id-keyed hook-registration entry point.
///
/// Both variants are configuration errors caught at registration time (cold
/// path); neither is reachable from the per-frame hot path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksError {
    /// The `HOOKS` slot for this id is already populated. The table is
    /// write-once (`OnceLock::set`): hooks are registered exactly once per
    /// `ComponentId` for the process lifetime, via EITHER the
    /// `#[component(...)]` derive OR a runtime registration — never both,
    /// never twice.
    AlreadyRegistered {
        /// The id whose slot was already occupied.
        component_id: ComponentId,
    },
    /// The id was already placed in at least one archetype of some world in
    /// this process (Phase-21 H1 staleness gate). That archetype's
    /// [`ArchetypeFlags`](crate::ecs::core::component::hooks::archetype_flags::ArchetypeFlags)
    /// were OR-computed without the hook bits, so the hooks would silently
    /// never fire there — the compile-but-lie class this gate exists to
    /// reject. Contract: *mint → register hooks → first attach*.
    AlreadyArchetyped {
        /// The id that already appeared in a live archetype.
        component_id: ComponentId,
    },
}

impl fmt::Display for HooksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRegistered { component_id } => write!(
                f,
                "hooks for {component_id} are already registered (the HOOKS table is \
                 write-once per id; derive `#[component(...)]` and runtime registration \
                 are mutually exclusive)"
            ),
            Self::AlreadyArchetyped { component_id } => write!(
                f,
                "{component_id} already appears in a live archetype of some world in this \
                 process (Phase-21 H1 staleness gate): its ArchetypeFlags were computed \
                 without the hook bits and the hooks would silently never fire. Register \
                 hooks before the first attach: mint -> register hooks -> first attach"
            ),
        }
    }
}

impl std::error::Error for HooksError {}

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
        assert!(h.on_despawn.is_none(), "Default on_despawn is None");
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
