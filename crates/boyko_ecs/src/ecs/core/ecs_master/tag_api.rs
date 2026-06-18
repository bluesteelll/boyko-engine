//! Phase 22 (D3/D9): the dynamic-tag surface on [`EcsMaster`].
//!
//! Two halves:
//!
//! * **Registration (D3)** — dynamic tags are process-global metadata, like
//!   every [`ComponentId`] (mirroring `LAYOUTS` / `HOOKS`): the minting
//!   methods delegate to the global intern in `component_registry.rs`. They
//!   live on `EcsMaster` (`&mut self` for the minting pair) so tag minting
//!   follows the same exclusive-world conventions as the rest of the
//!   structural API, and so a tag minted through ANY world is visible to ALL
//!   worlds.
//! * **Attach / detach / presence (D9)** — `add_tag` / `remove_tag` are
//!   per-world structural ops routed through the dynamic id-keyed migration
//!   helpers (`migration_helpers.rs`); `has_tag` is the O(1) entity-level
//!   presence probe (inland → archetype → signature bit test).
//!
//! [`ComponentId`]: crate::ecs::identifiers::primitives::ComponentId

use crate::ecs::core::commands::migration_helpers::{
    merged_archetype_id_dyn, migrate_entity_attach_ids, migrate_entity_detach_ids, retag_in_place,
    without_ids_archetype_id,
};
use crate::ecs::core::component::component_registry::{self, MAX_COMPONENTS, TagId};
use crate::ecs::core::component::hooks::scope::DeferredScopeGuard;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::entity::entity_inland::EntityInland;

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

    /// Returns `true` iff `entity` is live and its archetype hosts `tag`
    /// (Phase 22 D4/D9). Hot-capable O(1): inland load → archetype pointer →
    /// signature bit test (two dependent loads + one generation check; plan
    /// target ≤ 5 ns).
    ///
    /// `false` for dead / stale / never-registered entities — presence of a
    /// tag on a dead entity is not a meaningful question, mirroring
    /// `get_component`'s `None`.
    #[inline]
    pub fn has_tag(&self, entity: Entity, tag: TagId) -> bool {
        let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        let inland: EntityInland = *slot;
        if inland.is_null() || inland.generation() != entity.generation() {
            return false;
        }
        // SAFETY (U1, U2, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance minted at
        //   registration time — it survives sibling structural writes under
        //   TB/SB because the whole slab element is `UnsafeCell`-wrapped.
        //   Non-null + generation-matched above ⇒ the slot is live. The read
        //   is a shared signature-word load (no `&mut` taken).
        unsafe { (*inland.archetype_ptr()).has_component_id(tag.component_id()) }
    }

    /// Attaches the dynamic tag `tag` to `entity` (Phase 22 D9). Direct,
    /// migrating structural op (`&mut self` — structural window or
    /// apply-window barrier, like every structural op).
    ///
    /// Semantics (plan D8/D9):
    ///
    /// * **Absent tag** — archetype migration `source → source ∪ {tag}`
    ///   through `migrate_entity_attach_ids`; `on_add` + `on_insert` hooks
    ///   and observers fire for the tag. Attaching to an empty entity routes
    ///   it out of the EMPTY archetype (zero-retained shape, O3).
    /// * **Present tag** — in-place replace semantics: `on_replace` +
    ///   `on_insert` fire and the changed tick is stamped (uniform with data
    ///   replace; `on_add` does NOT fire). No migration.
    /// * **Dead / stale entity** — silent no-op (matching the deferred
    ///   command contract: a despawn may legitimately race an enqueued tag
    ///   op within a frame).
    ///
    /// Cost: `#[cold]` migration (row move + tick init) on first attach per
    /// archetype shape; tag columns themselves copy zero bytes.
    pub fn add_tag(&mut self, entity: Entity, tag: TagId) {
        let cid = tag.component_id();

        // Resolve the fast inland by value; dead / stale ⇒ silent no-op.
        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return;
            }
            *slot
        };

        // Phase 14a §3.6 / §8 P1: RAII depth bracket — hooks fired by the
        // migration below may enqueue deferred commands; only the outermost
        // owner drains (Q-A1). `Drop` restores the depth on every exit.
        let scope = DeferredScopeGuard::enter();

        // SAFETY (U1, U2, U11, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance — it survives
        //   sibling structural writes under TB/SB (whole slab element is
        //   `UnsafeCell`-wrapped). Non-null + generation-matched above ⇒ live.
        //   Shared reads only (`id`, signature word); no `&mut` taken.
        let (source_archetype_id, present) = unsafe {
            let archetype = &*inland.archetype_ptr();
            (archetype.id(), archetype.has_component_id(cid))
        };

        if present {
            // D8: re-inserting a present tag is in-place replace semantics.
            retag_in_place(self, entity, &[cid]);
        } else {
            let target_archetype_id = merged_archetype_id_dyn(self, source_archetype_id, &[cid]);
            // `cid ∉ source` ⇒ the union is strictly larger than the source
            // set ⇒ a distinct exact-mask match.
            debug_assert_ne!(
                target_archetype_id, source_archetype_id,
                "merged_archetype_id_dyn returned the source for a strictly-growing union"
            );
            migrate_entity_attach_ids(
                self,
                entity,
                source_archetype_id,
                target_archetype_id,
                &[cid],
            );
        }

        // Direct API: drop the bracket (depth back down), then drain. At
        // depth 0 (direct call) the drain runs; reached from
        // `AddTagCommand::apply` at depth >= 1 it no-ops and the outermost
        // owner drains (Q-A1) — mirrors `delete_entity`.
        drop(scope);
        self.drain_deferred_hook_queue();
    }

    /// Detaches the dynamic tag `tag` from `entity` (Phase 22 D9). Direct,
    /// migrating structural op.
    ///
    /// Semantics (plan D9):
    ///
    /// * **Present tag** — archetype migration `source → source \ {tag}`
    ///   through `migrate_entity_detach_ids`; `on_replace` + `on_remove`
    ///   hooks and observers fire for the tag against the dying source row.
    ///   Removing the last component routes the entity INTO the EMPTY
    ///   archetype (O3) — the entity stays alive with zero components.
    /// * **Absent tag** — silent no-op (W1 — Bevy Issue #10166 parity with
    ///   `remove::<C>()`).
    /// * **Dead / stale entity** — silent no-op.
    pub fn remove_tag(&mut self, entity: Entity, tag: TagId) {
        let cid = tag.component_id();

        let inland: EntityInland = {
            let Some(slot) = self.entity_master.entities_inland.get(entity.id().0) else {
                return;
            };
            if slot.is_null() || slot.generation() != entity.generation() {
                return;
            }
            *slot
        };

        // SAFETY: same rationale as in `add_tag` — shared reads through stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance;
        //   non-null + generation-matched ⇒ live; no `&mut` taken.
        let (source_archetype_id, present) = unsafe {
            let archetype = &*inland.archetype_ptr();
            (archetype.id(), archetype.has_component_id(cid))
        };
        if !present {
            return; // W1: absent tag ⇒ silent no-op (decided on the signature)
        }

        // RAII depth bracket + drain — same discipline as `add_tag`.
        let scope = DeferredScopeGuard::enter();

        let target_archetype_id = without_ids_archetype_id(self, source_archetype_id, &[cid]);
        // `cid ∈ source` ⇒ `kept` is strictly smaller ⇒ a distinct exact-mask
        // match (the EMPTY archetype when the tag was the last component).
        debug_assert_ne!(
            target_archetype_id, source_archetype_id,
            "without_ids_archetype_id returned the source for a strictly-shrinking set"
        );
        migrate_entity_detach_ids(self, entity, source_archetype_id, target_archetype_id, &[cid]);

        drop(scope);
        self.drain_deferred_hook_queue();
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
