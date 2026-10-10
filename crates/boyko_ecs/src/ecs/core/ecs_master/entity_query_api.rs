//! Entity metadata & id-list query surface on [`EcsMaster`] (mechanical split).
//!
//! Liveness / archetype / count probes plus the `query_entities*` id-list
//! helpers. Extracted verbatim from `ecs_master.rs`.

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::identifiers::primitives::{
    ArchetypeId, ComponentId, EntityId, InlandPoolId,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    /// Fast existence check: 1 cache line, ~5 ns target. Returns `true`
    /// iff the slot for `entity.id()` is live AND its stored generation
    /// matches the handle.
    #[inline]
    pub fn has_entity(&self, entity: Entity) -> bool {
        let Some(inland) = self.entity_master.entities_inland.get(entity.id().0) else {
            return false;
        };
        !inland.is_null() && inland.generation() == entity.generation()
    }

    /// Gets an entity by ID if it exists and is active
    #[inline]
    pub fn get_entity(&self, entity_id: EntityId) -> Option<Entity> {
        self.entity_master.get_entity(entity_id)
    }

    /// Returns `entity`'s current archetype id, or `None` for a stale /
    /// never-registered handle. The stable identity used to assert the Dense
    /// plan D2 "no-migration" contract (a dense insert/remove leaves this id
    /// unchanged).
    #[inline]
    pub fn entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        // BUG-MIGRATE-TB-1: raw projection of `id` (no `&Archetype` foreign read
        // that would freeze a concurrently sibling-written `current_index`).
        // SAFETY (U1, U2, U11, F1): `archetype_ptr` is stable, interior-mutable
        //   (`SharedReadWrite`, F4-rooted) slab provenance; reading `id` is one
        //   load through a raw projection.
        Some(unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).id).read() })
    }

    /// Gets the archetype ID containing the specified entity.
    ///
    /// Derives the id from the fast inland's slab pointer via
    /// `Archetype::id` — no SparseMap traversal.
    #[inline]
    pub fn get_entity_archetype_id(&self, entity: Entity) -> Option<ArchetypeId> {
        let inland = self.entity_master.entities_inland.get(entity.id().0)?;
        if inland.is_null() || inland.generation() != entity.generation() {
            return None;
        }
        // BUG-MIGRATE-TB-1: read `id` through a raw projection (no `&Archetype`)
        // so a concurrent sibling `current_index` write is not frozen by this
        // foreign read. `id` is `Copy`.
        // SAFETY (U1, U2, U11, F1): same as get_component_raw — stable,
        //   interior-mutable (`SharedReadWrite`, F4-rooted) slab provenance.
        let id = unsafe { core::ptr::addr_of!((*inland.archetype_ptr()).id).read() };
        Some(id)
    }

    /// Gets the total number of active entities in the system
    #[inline]
    pub fn entity_count(&self) -> usize {
        self.entity_master.entity_count()
    }

    /// Gets the number of archetypes in the system
    #[inline]
    pub fn archetype_count(&self) -> usize {
        self.archetype_master.archetype_count()
    }

    /// Gets the number of recycled entity IDs available for reuse
    #[inline]
    pub fn recycled_entity_count(&self) -> usize {
        self.entity_master.recycled_entity_count()
    }

    /// Gets an iterator over all active entities
    #[inline]
    pub fn iter_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entity_master.iter_entities()
    }

    /// Queries entities that have all specified components
    pub fn query_entities(&self, component_ids: &[ComponentId]) -> Vec<Entity> {
        let mut result = Vec::new();
        self.query_entities_into(component_ids, &mut result);
        result
    }

    /// Writes every entity hosting all of `component_ids` into `out`, reusing
    /// `arch_scratch` for the matching-archetype id list.
    ///
    /// # API contract
    /// BOTH `out` and `arch_scratch` are **cleared at function entry**; their
    /// existing contents are discarded and only their capacity is reused. This is
    /// the fully allocation-free query primitive: the per-frame UI
    /// interaction/bind walks drive it through two retained scratch buffers so the
    /// steady-state path allocates NOTHING (Principle 1/5, the plan's "0
    /// allocations/frame" mandate).
    pub fn query_entities_buf(
        &self,
        component_ids: &[ComponentId],
        out: &mut Vec<Entity>,
        arch_scratch: &mut Vec<ArchetypeId>,
    ) {
        out.clear();
        self.archetype_master
            .find_archetypes_with_components_into(component_ids, arch_scratch);
        for &archetype_id in arch_scratch.iter() {
            if let Some(archetype) = self.archetype_master.get_archetype(archetype_id) {
                for unit_index in 0..archetype.entity_count() {
                    if let Some(entity_id) = archetype.get_entity_id_at(InlandPoolId(unit_index))
                        && let Some(entity) = self.entity_master.get_entity(entity_id)
                    {
                        out.push(entity);
                    }
                }
            }
        }
    }

    /// Writes every entity hosting all of `component_ids` into `out` (clears `out`
    /// first). Convenience wrapper over [`query_entities_buf`](Self::query_entities_buf)
    /// with a transient archetype-id buffer; for the allocation-free per-frame
    /// path use `query_entities_buf` with a retained scratch.
    #[inline]
    pub fn query_entities_into(&self, component_ids: &[ComponentId], out: &mut Vec<Entity>) {
        let mut arch_scratch = Vec::new();
        self.query_entities_buf(component_ids, out, &mut arch_scratch);
    }

}
