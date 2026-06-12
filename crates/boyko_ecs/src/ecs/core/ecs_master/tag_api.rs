//! Phase 22 (D3): the dynamic-tag registration surface on [`EcsMaster`].
//!
//! Dynamic tags are process-global metadata, like every [`ComponentId`]
//! (mirroring `LAYOUTS` / `HOOKS`): the methods here delegate to the global
//! intern in `component_registry.rs`. They live on `EcsMaster` (`&mut self`
//! for the minting pair) so tag minting follows the same exclusive-world
//! conventions as the rest of the structural API, and so a tag minted through
//! ANY world is visible to ALL worlds.
//!
//! [`ComponentId`]: crate::ecs::identifiers::primitives::ComponentId

use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS, TagId};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    /// Mints (or resolves) the dynamic tag named `name` (Phase 22 D3).
    ///
    /// Fallible-first by design: dynamic mints are user-data-driven (names
    /// from config/scripts), so the budget panic must be opt-in (see
    /// [`register_tag`](Self::register_tag)).
    ///
    /// - Idempotent per name: the same `name` always returns the same
    ///   [`TagId`], including after the budget is exhausted (an interned name
    ///   is a success, never `None`).
    /// - `None`: the shared `MAX_COMPONENTS` (512) ComponentId budget —
    ///   shared with every typed component — is exhausted and `name` was
    ///   never minted.
    ///
    /// The numeric id is first-call-order process-unstable; the **name** is
    /// the stable key. Registration is cold (lock + hash map); never call it
    /// on the per-frame hot path — mint once at setup and keep the [`TagId`].
    #[cold]
    pub fn try_register_tag(&mut self, name: &str) -> Option<TagId> {
        component_registry::try_register_tag_by_name(name)
    }

    /// Panicking sugar over [`try_register_tag`](Self::try_register_tag)
    /// (Phase 22 D3).
    ///
    /// # Hook-registration contract (Phase-21 H1)
    ///
    /// Register lifecycle hooks for a tag BETWEEN minting it and its first
    /// attach: *mint → register hooks → first attach* (see
    /// [`register_hooks_by_id`](crate::ecs::core::component::component_registry::register_hooks_by_id)).
    ///
    /// # Panics
    ///
    /// If the shared `MAX_COMPONENTS` (512) ComponentId budget is exhausted
    /// and `name` was never minted.
    #[cold]
    pub fn register_tag(&mut self, name: &str) -> TagId {
        match component_registry::try_register_tag_by_name(name) {
            Some(tag) => tag,
            None => register_tag_exhausted_panic(name),
        }
    }

    /// Resolves a previously minted dynamic tag by name (Phase 22 D3). Cold
    /// lookup; never mints. `None` if `name` was never successfully minted in
    /// this process.
    #[cold]
    pub fn tag_by_name(&self, name: &str) -> Option<TagId> {
        component_registry::tag_by_name(name)
    }
}

/// Cold panic site for [`EcsMaster::register_tag`] at budget exhaustion,
/// naming the shared 512-slot budget (plan D3).
#[cold]
#[inline(never)]
fn register_tag_exhausted_panic(name: &str) -> ! {
    panic!(
        "register_tag(\"{name}\"): the shared component-id budget is exhausted — dynamic \
         tags share the {MAX_COMPONENTS}-slot ComponentId space with typed components. \
         Use try_register_tag for a fallible mint."
    );
}
