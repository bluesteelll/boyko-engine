//! Per-system aliasing summary.
//!
//! [`Access`] is the read/write surface declared by a system's `SystemParam`
//! collection. Phase 8a populates it during `init_access` (see
//! [`FilteredAccessSet`]); Phase 9 scheduler reads it to build the conflict
//! graph between systems. It is **write-once**: once finalized into a
//! `SystemMeta`, it is read-only forever.
//!
//! # Layout (M1 RESOLUTION)
//!
//! 192 B total. Natural alignment only — no `align(64)`, no `_pad`.
//! Rationale: `Access` is written exclusively during single-threaded
//! `init_access` and read concurrently by the scheduler thereafter; there
//! is no false-sharing risk to defend against. Phase 9 will re-evaluate if
//! per-thread mutation becomes a requirement.
//!
//! # Predicate split (M8 RESOLUTION)
//!
//! [`Access::conflicts_with`] is the **cross-system** predicate (Phase 9
//! scheduler). It returns `true` if two systems cannot run concurrently.
//! **Intra-system** conflict detection (e.g. `Res<X> + ResMut<X>` in one
//! system's params) is the responsibility of [`FilteredAccessSet`], which
//! accumulates per-bit ownership during init.
//!
//! [`FilteredAccessSet`]: super::FilteredAccessSet

use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};
use boyko_utils::bit_mask::bit_set_256::BitSet256;

/// Read/write surface declared by a system's parameters.
///
/// Mutated only during `SystemParam::init_access`; read-only thereafter.
/// See module docs for layout rationale and the [`conflicts_with`] split
/// from [`FilteredAccessSet`].
///
/// # Forward compatibility (`#[non_exhaustive]`)
///
/// Marked `#[non_exhaustive]` because Phase 9 (event-bus mask) and
/// Phase 10 (tick-scope channels) will extend the surface. Downstream
/// matchers on `Access` must include a `..` rest pattern.
///
/// [`conflicts_with`]: Access::conflicts_with
/// [`FilteredAccessSet`]: super::FilteredAccessSet
#[repr(C)]
#[non_exhaustive]
pub struct Access {
    /// Components this system reads. 64 B ([`ComponentMask`] = 512 bits).
    pub(crate) component_reads: ComponentMask,
    /// Components this system writes. 64 B.
    pub(crate) component_writes: ComponentMask,
    /// Resources this system reads. 32 B ([`BitSet256`]).
    pub(crate) resource_reads: BitSet256,
    /// Resources this system writes. 32 B.
    pub(crate) resource_writes: BitSet256,
    // 64 + 64 + 32 + 32 = 192 B, 3 cache lines, no padding.
}

// Plan §4.1 + §11.1: `Access` must remain 192 B exactly. Bumping the layout
// silently inflates per-system memory and breaks the 3-cache-line claim.
const _: () = assert!(core::mem::size_of::<Access>() == 192);

impl Access {
    /// Constructs an empty access surface — no reads, no writes.
    ///
    /// Not `const` because [`ComponentMask::new`] is not `const`. If
    /// `ComponentMask` later gains a `const fn new`, this constructor can
    /// be promoted without changing any call sites.
    #[inline]
    pub fn new() -> Self {
        Self {
            component_reads: ComponentMask::new(),
            component_writes: ComponentMask::new(),
            resource_reads: BitSet256::new(),
            resource_writes: BitSet256::new(),
        }
    }

    /// Declares that the system reads the component identified by `id`.
    #[inline]
    pub fn add_component_read(&mut self, id: ComponentId) {
        self.component_reads.set(id);
    }

    /// Declares that the system writes the component identified by `id`.
    #[inline]
    pub fn add_component_write(&mut self, id: ComponentId) {
        self.component_writes.set(id);
    }

    /// Declares that the system reads the resource identified by `id`.
    #[inline]
    pub fn add_resource_read(&mut self, id: ResourceId) {
        self.resource_reads.set(id.0);
    }

    /// Declares that the system writes the resource identified by `id`.
    #[inline]
    pub fn add_resource_write(&mut self, id: ResourceId) {
        self.resource_writes.set(id.0);
    }

    /// **Cross-system** conflict check (Phase 9 scheduler use only).
    ///
    /// Returns `true` iff `self` and `other` cannot execute concurrently.
    /// The check covers all four read/write × component/resource crossings.
    ///
    /// # M8 — intra-system check is elsewhere
    ///
    /// `self.conflicts_with(self)` trivially returns `true` when `self` has
    /// any write — that is correct for the cross-system case (a system
    /// cannot run twice against itself in parallel). Detecting `Res<X>` and
    /// `ResMut<X>` *within the same system's parameter list* is the job of
    /// [`FilteredAccessSet`], which tracks per-bit ownership during init.
    ///
    /// [`FilteredAccessSet`]: super::FilteredAccessSet
    pub fn conflicts_with(&self, other: &Access) -> bool {
        let cw_vs_cr = self.component_writes.intersects(&other.component_reads);
        let cr_vs_cw = self.component_reads.intersects(&other.component_writes);
        let cw_vs_cw = self.component_writes.intersects(&other.component_writes);
        let rw_vs_rr = self.resource_writes.intersects(&other.resource_reads);
        let rr_vs_rw = self.resource_reads.intersects(&other.resource_writes);
        let rw_vs_rw = self.resource_writes.intersects(&other.resource_writes);
        cw_vs_cr || cr_vs_cw || cw_vs_cw || rw_vs_rr || rr_vs_rw || rw_vs_rw
    }
}

impl Default for Access {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two disjoint accesses do not conflict in either direction.
    #[test]
    fn conflicts_with_disjoint_false() {
        let mut a = Access::new();
        let mut b = Access::new();
        a.add_resource_read(ResourceId(0));
        a.add_resource_write(ResourceId(1));
        b.add_resource_read(ResourceId(2));
        b.add_resource_write(ResourceId(3));
        assert!(!a.conflicts_with(&b), "disjoint resource access must not conflict");
        assert!(!b.conflicts_with(&a), "conflict relation is symmetric");
    }

    /// Two writes to the same resource conflict regardless of side.
    #[test]
    fn conflicts_with_overlapping_write_true() {
        let mut a = Access::new();
        let mut b = Access::new();
        a.add_resource_write(ResourceId(7));
        b.add_resource_write(ResourceId(7));
        assert!(a.conflicts_with(&b), "double-write on the same resource must conflict");
        assert!(b.conflicts_with(&a), "conflict relation is symmetric");
    }

    /// Adding a read sets the corresponding bit so `conflicts_with` against
    /// a writer of the same id reports the conflict.
    #[test]
    fn add_read_then_check() {
        let mut reader = Access::new();
        let mut writer = Access::new();
        reader.add_resource_read(ResourceId(5));
        writer.add_resource_write(ResourceId(5));
        assert!(
            reader.conflicts_with(&writer),
            "reader-vs-writer on the same resource must conflict"
        );
        assert!(
            writer.conflicts_with(&reader),
            "writer-vs-reader on the same resource must conflict"
        );
    }

    /// Two readers on the same resource do not conflict.
    #[test]
    fn two_reads_no_conflict() {
        let mut a = Access::new();
        let mut b = Access::new();
        a.add_resource_read(ResourceId(5));
        b.add_resource_read(ResourceId(5));
        assert!(!a.conflicts_with(&b), "double-read must not conflict");
    }

    /// Component conflicts are detected independently of resource conflicts.
    #[test]
    fn component_conflict_independent_of_resource() {
        let mut a = Access::new();
        let mut b = Access::new();
        a.add_component_write(ComponentId(3));
        b.add_component_read(ComponentId(3));
        assert!(a.conflicts_with(&b), "component write-vs-read must conflict");
        assert!(b.conflicts_with(&a));
    }

    /// `Default` matches `new` — empty access.
    #[test]
    fn default_is_empty() {
        let a = Access::default();
        let b = Access::default();
        assert!(!a.conflicts_with(&b), "empty access never conflicts");
    }
}
