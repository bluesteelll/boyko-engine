//! Intra-system aliasing detector — accumulates per-bit ownership so
//! sibling `SystemParam`s reject conflicting access at registration time.
//!
//! See Phase 8a plan §4.5 (C4 + M8 RESOLUTION) and §4.6 (W2 RESOLUTION).
//!
//! # Why a separate struct
//!
//! [`Access`] is the lean **summary** stored on `SystemMeta` (192 B). It has
//! no room for per-bit ownership tracking, which is needed only during the
//! brief `init_access` window. [`FilteredAccessSet`] carries that heavy
//! per-bit map (24 KB heap, transient) and discards it via [`finalize`],
//! copying only the accumulated [`Access`] back into the meta.
//!
//! # Indexing
//!
//! `bit_owners` is a single contiguous `&'static str` array indexed by:
//!
//! | Range | Meaning |
//! |-------|---------|
//! | `0..512`     | component reads (by `ComponentId.0`) |
//! | `512..1024`  | component writes |
//! | `1024..1280` | resource reads (by `ResourceId.0`) |
//! | `1280..1536` | resource writes |
//!
//! [`finalize`]: FilteredAccessSet::finalize

use crate::ecs::core::component::component_registry::MAX_COMPONENTS;
use crate::ecs::core::resources::resource_registry::RESOURCE_SLOT_COUNT;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};

// ── Index layout constants ──────────────────────────────────────────────────

/// Total number of ownership slots tracked by [`FilteredAccessSet`].
///
/// 512 (component reads) + 512 (component writes) + 256 (resource reads)
/// + 256 (resource writes) = **1536**.
///
/// The chosen sizes mirror [`ComponentMask`]'s 512-bit width and
/// [`BitSet256`]'s 256-bit width so the index ranges align 1:1 with the
/// bits each `Access` field can address.
///
/// [`ComponentMask`]: crate::ecs::core::component::component_mask::ComponentMask
/// [`BitSet256`]: boyko_utils::bit_mask::bit_set_256::BitSet256
pub const OWNERSHIP_SLOT_COUNT: usize = 1536;

/// Offset of the component-reads range in `bit_owners`.
const COMPONENT_READ_BASE: usize = 0;
/// Offset of the component-writes range in `bit_owners`.
const COMPONENT_WRITE_BASE: usize = 512;
/// Offset of the resource-reads range in `bit_owners`.
const RESOURCE_READ_BASE: usize = 1024;
/// Offset of the resource-writes range in `bit_owners`.
const RESOURCE_WRITE_BASE: usize = 1280;

// Sanity: the four ranges abut and the total equals OWNERSHIP_SLOT_COUNT.
const _: () =
    assert!(COMPONENT_WRITE_BASE - COMPONENT_READ_BASE == MAX_COMPONENTS);
const _: () =
    assert!(RESOURCE_READ_BASE - COMPONENT_WRITE_BASE == MAX_COMPONENTS);
const _: () =
    assert!(RESOURCE_WRITE_BASE - RESOURCE_READ_BASE == RESOURCE_SLOT_COUNT);
const _: () =
    assert!(OWNERSHIP_SLOT_COUNT - RESOURCE_WRITE_BASE == RESOURCE_SLOT_COUNT);

// ── Conflict diagnostic ────────────────────────────────────────────────────

/// Why two `SystemParam`s within one system cannot coexist.
///
/// Carried inside [`AccessConflict`]; consumed by the param-side panic
/// shim (B0002 diagnostic — see Phase 8a plan §4.5 example output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// New param wants to read a resource that a sibling already writes.
    ResourceReadVsWrite,
    /// New param wants to write a resource that a sibling already reads.
    ResourceWriteVsRead,
    /// New param wants to write a resource that a sibling already writes.
    ResourceWriteVsWrite,
    /// New param wants to read a component that a sibling already writes.
    ComponentReadVsWrite,
    /// New param wants to write a component that a sibling already reads.
    ComponentWriteVsRead,
    /// New param wants to write a component that a sibling already writes.
    ComponentWriteVsWrite,
}

/// Description of an intra-system access conflict, returned from the
/// `FilteredAccessSet::add_*` methods.
///
/// Carried by `Err` into each `SystemParam::init_access` and ultimately
/// surfaced as a B0002 panic diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessConflict {
    /// What kind of conflict occurred.
    pub kind: ConflictKind,
    /// Raw `ComponentId.0` or `ResourceId.0` of the offending resource.
    pub id: usize,
    /// `std::any::type_name` of the param that registered the conflicting
    /// bit FIRST (was already in the set).
    pub existing_param: &'static str,
    /// `std::any::type_name` of the param that tried to add the bit and got
    /// rejected.
    pub new_param: &'static str,
}

// ── FilteredAccessSet ──────────────────────────────────────────────────────

/// Per-system accumulator for `SystemParam::init_access`.
///
/// Holds:
///   * `combined`: the running [`Access`] summary so far.
///   * `bit_owners`: a heap-allocated 24 KB ownership map indexed per
///     the layout above (`OWNERSHIP_SLOT_COUNT` slots × 16 B fat pointer
///     per `&'static str` = 24 KB — W2 RESOLUTION).
///
/// `bit_owners` lives on the heap (`Box<[...; 1536]>`) so the stack
/// footprint of the per-system init frame stays bounded (200 B), and the
/// 24 KB allocation is freed by [`finalize`] after init completes.
#[repr(C)]
pub struct FilteredAccessSet {
    /// Running aggregate of all params declared so far in this system.
    combined: Access,
    /// Per-slot ownership map. Heap-allocated to keep stack frames small;
    /// 24 KB transient allocation per system per init call.
    bit_owners: Box<[&'static str; OWNERSHIP_SLOT_COUNT]>,
}

impl FilteredAccessSet {
    /// Constructs an empty accumulator.
    ///
    /// Allocates the 24 KB `bit_owners` slab on the heap; the slab is
    /// freed by [`finalize`] once init completes.
    #[cold]
    pub fn new() -> Self {
        Self {
            combined: Access::new(),
            bit_owners: Box::new([""; OWNERSHIP_SLOT_COUNT]),
        }
    }

    /// Declares that the active param reads the resource identified by
    /// `id`. Returns `Err` if a sibling param already declared a *write*
    /// to the same resource.
    pub fn add_resource_read(
        &mut self,
        id: ResourceId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0;
        debug_assert!(idx < RESOURCE_SLOT_COUNT, "ResourceId out of range: {idx}");
        if self.combined.resource_writes.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceReadVsWrite,
                id: idx,
                existing_param: self.bit_owners[RESOURCE_WRITE_BASE + idx],
                new_param: param_name,
            });
        }
        self.combined.resource_reads.set(idx);
        self.bit_owners[RESOURCE_READ_BASE + idx] = param_name;
        Ok(())
    }

    /// Declares that the active param writes the resource identified by
    /// `id`. Returns `Err` if a sibling param already declared *any*
    /// access (read or write) to the same resource.
    pub fn add_resource_write(
        &mut self,
        id: ResourceId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0;
        debug_assert!(idx < RESOURCE_SLOT_COUNT, "ResourceId out of range: {idx}");
        if self.combined.resource_reads.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceWriteVsRead,
                id: idx,
                existing_param: self.bit_owners[RESOURCE_READ_BASE + idx],
                new_param: param_name,
            });
        }
        if self.combined.resource_writes.get(idx) {
            return Err(AccessConflict {
                kind: ConflictKind::ResourceWriteVsWrite,
                id: idx,
                existing_param: self.bit_owners[RESOURCE_WRITE_BASE + idx],
                new_param: param_name,
            });
        }
        self.combined.resource_writes.set(idx);
        self.bit_owners[RESOURCE_WRITE_BASE + idx] = param_name;
        Ok(())
    }

    /// Declares that the active param reads the component identified by
    /// `id`. Returns `Err` if a sibling param already declared a *write*
    /// to the same component.
    pub fn add_component_read(
        &mut self,
        id: ComponentId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0;
        debug_assert!(idx < MAX_COMPONENTS, "ComponentId out of range: {idx}");
        if self.combined.component_writes.contains(id) {
            return Err(AccessConflict {
                kind: ConflictKind::ComponentReadVsWrite,
                id: idx,
                existing_param: self.bit_owners[COMPONENT_WRITE_BASE + idx],
                new_param: param_name,
            });
        }
        self.combined.component_reads.set(id);
        self.bit_owners[COMPONENT_READ_BASE + idx] = param_name;
        Ok(())
    }

    /// Declares that the active param writes the component identified by
    /// `id`. Returns `Err` if a sibling param already declared *any*
    /// access (read or write) to the same component.
    pub fn add_component_write(
        &mut self,
        id: ComponentId,
        param_name: &'static str,
    ) -> Result<(), AccessConflict> {
        let idx = id.0;
        debug_assert!(idx < MAX_COMPONENTS, "ComponentId out of range: {idx}");
        if self.combined.component_reads.contains(id) {
            return Err(AccessConflict {
                kind: ConflictKind::ComponentWriteVsRead,
                id: idx,
                existing_param: self.bit_owners[COMPONENT_READ_BASE + idx],
                new_param: param_name,
            });
        }
        if self.combined.component_writes.contains(id) {
            return Err(AccessConflict {
                kind: ConflictKind::ComponentWriteVsWrite,
                id: idx,
                existing_param: self.bit_owners[COMPONENT_WRITE_BASE + idx],
                new_param: param_name,
            });
        }
        self.combined.component_writes.set(id);
        self.bit_owners[COMPONENT_WRITE_BASE + idx] = param_name;
        Ok(())
    }

    /// Declares that the active param requires **universal access** — it
    /// reads and writes every component and every resource (Phase 4 CR-B).
    ///
    /// Used by the NonSend `SystemParam`s (`NonSendRes` / `NonSendResMut`):
    /// declaring universal access makes `meta.access().is_universal()` true
    /// after [`finalize`], so the existing `SystemKind` derivation resolves
    /// the system to `CpuExclusive` and the conflict graph serializes it —
    /// the NonSend payload is then touched only on the dispatcher when
    /// `running == 0` (the apply-window single-thread-touch invariant).
    ///
    /// Unlike `add_resource_*` / `add_component_*` this cannot conflict with
    /// a sibling param's prior bits (it is a superset), so it returns no
    /// `Result`. Per-bit `bit_owners` ownership is NOT tracked for the
    /// universal grant — universal access already conflicts with everything
    /// cross-system, and a NonSend system is dispatcher-solo regardless.
    ///
    /// [`finalize`]: FilteredAccessSet::finalize
    pub fn mark_universal(&mut self) {
        self.combined = Access::universal();
    }

    /// Returns a shared view of the accumulated [`Access`] so far.
    /// Intended for inspection during init (e.g. debug logging).
    #[inline]
    pub fn combined(&self) -> &Access {
        &self.combined
    }

    /// Consumes the accumulator, moving the finalized [`Access`] into
    /// `meta.access`. The 24 KB `bit_owners` slab is freed by this call's
    /// drop of `self`.
    #[inline]
    pub fn finalize(self, meta: &mut SystemMeta) {
        meta.access = self.combined;
    }
}

impl Default for FilteredAccessSet {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two disjoint reads are always compatible.
    #[test]
    fn add_two_disjoint_reads_ok() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_read(ResourceId(0), "Res<A>")
            .expect("first read must succeed");
        set.add_resource_read(ResourceId(1), "Res<B>")
            .expect("disjoint second read must succeed");
    }

    /// Two reads of the *same* resource are compatible (both shared).
    #[test]
    fn add_res_res_same_id_ok() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_read(ResourceId(7), "Res<A>")
            .expect("first read must succeed");
        set.add_resource_read(ResourceId(7), "Res<A> (second site)")
            .expect("two reads of the same resource must be compatible");
    }

    /// `ResMut<X>` after `ResMut<X>` on the same id is a double-write.
    #[test]
    fn add_resmut_resmut_same_id_conflict_err() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_write(ResourceId(3), "ResMut<A>")
            .expect("first write must succeed");
        let conflict = set
            .add_resource_write(ResourceId(3), "ResMut<A> (sibling)")
            .expect_err("double-write on the same resource must fail");
        assert_eq!(conflict.kind, ConflictKind::ResourceWriteVsWrite);
        assert_eq!(conflict.id, 3);
        assert_eq!(conflict.existing_param, "ResMut<A>");
        assert_eq!(conflict.new_param, "ResMut<A> (sibling)");
    }

    /// `Res<X>` after `ResMut<X>` on the same id is a read-vs-write
    /// conflict.
    #[test]
    fn add_res_resmut_same_id_conflict_err() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_write(ResourceId(5), "ResMut<A>")
            .expect("write must succeed");
        let conflict = set
            .add_resource_read(ResourceId(5), "Res<A>")
            .expect_err("read after write on the same resource must fail");
        assert_eq!(conflict.kind, ConflictKind::ResourceReadVsWrite);
        assert_eq!(conflict.id, 5);
        assert_eq!(conflict.existing_param, "ResMut<A>");
        assert_eq!(conflict.new_param, "Res<A>");
    }

    /// `ResMut<X>` after `Res<X>` on the same id is a write-vs-read
    /// conflict (mirror of the previous test).
    #[test]
    fn add_resmut_after_res_same_id_conflict_err() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_read(ResourceId(9), "Res<A>")
            .expect("read must succeed");
        let conflict = set
            .add_resource_write(ResourceId(9), "ResMut<A>")
            .expect_err("write after read on the same resource must fail");
        assert_eq!(conflict.kind, ConflictKind::ResourceWriteVsRead);
        assert_eq!(conflict.id, 9);
        assert_eq!(conflict.existing_param, "Res<A>");
        assert_eq!(conflict.new_param, "ResMut<A>");
    }

    /// Two component reads of the same id are compatible.
    #[test]
    fn add_component_reads_compatible() {
        let mut set = FilteredAccessSet::new();
        set.add_component_read(ComponentId(11), "Q<&A>")
            .expect("first read must succeed");
        set.add_component_read(ComponentId(11), "Q<&A> (sibling)")
            .expect("two reads of the same component must be compatible");
    }

    /// Component read after component write is a conflict.
    #[test]
    fn add_component_read_after_write_conflict_err() {
        let mut set = FilteredAccessSet::new();
        set.add_component_write(ComponentId(13), "Q<&mut A>")
            .expect("write must succeed");
        let conflict = set
            .add_component_read(ComponentId(13), "Q<&A>")
            .expect_err("read-vs-write on same component must conflict");
        assert_eq!(conflict.kind, ConflictKind::ComponentReadVsWrite);
        assert_eq!(conflict.id, 13);
    }

    /// `finalize` moves the accumulated `Access` into the `SystemMeta` and
    /// the values round-trip via `meta.access()`.
    #[test]
    fn finalize_moves_combined_into_meta() {
        let mut set = FilteredAccessSet::new();
        set.add_resource_read(ResourceId(2), "Res<X>")
            .expect("read must succeed");
        set.add_resource_write(ResourceId(6), "ResMut<Y>")
            .expect("write must succeed");

        let mut meta = SystemMeta::for_testing("finalize_test");
        set.finalize(&mut meta);

        // Construct a probe access that conflicts only if the meta carries
        // the expected bits.
        let mut probe_write_x = Access::new();
        probe_write_x.add_resource_write(ResourceId(2));
        assert!(
            meta.access().conflicts_with(&probe_write_x),
            "finalized meta must carry the Res<X> read bit"
        );

        let mut probe_read_y = Access::new();
        probe_read_y.add_resource_read(ResourceId(6));
        assert!(
            meta.access().conflicts_with(&probe_read_y),
            "finalized meta must carry the ResMut<Y> write bit"
        );

        // Untouched resource has no bit set — no conflict.
        let mut probe_unrelated = Access::new();
        probe_unrelated.add_resource_write(ResourceId(100));
        assert!(
            !meta.access().conflicts_with(&probe_unrelated),
            "finalized meta must not carry spurious bits"
        );
    }
}
