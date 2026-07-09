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

    /// Returns `true` iff this access surface covers the entire component AND
    /// resource space — i.e. every bit in all four bitmasks is set.
    ///
    /// Used by the Phase 9 scheduler to identify **exclusive systems** (those
    /// requiring `&mut EcsMaster`, e.g. `ApplyDeferred`). A `SystemBox` caches
    /// the result at build time as `SystemKind::CpuExclusive` (Phase 4 D5;
    /// plan §2.5 EXC2, §13.6 SCH15 debug assert). Universal access conflicts
    /// with every other non-empty access in `conflicts_with`, so the conflict
    /// graph naturally serializes such systems against the rest.
    ///
    /// # Events outside the conflict graph (plan §2.2 SCH7 / §12.5 / §16 R)
    ///
    /// **Events do not participate in `Access`.** Per the EVT1 invariant
    /// (Phase 9 §2.8), each worker writes to its own `EventDispatcher` lane
    /// via TLS routing; the dispatcher writes to lane `worker_count`. No two
    /// systems alias an event lane mutably, so event access never needs to be
    /// reflected here. `ApplyDeferred` (whose access *is* universal) still
    /// blocks every other system equally, and `EventDispatcher::send_event<E>`
    /// picks the correct lane through TLS — no `Access`-level coordination is
    /// required. Adding `event_*` bitmasks would extend `Access` beyond 192 B
    /// and break the 3-cache-line invariant asserted on line 61.
    #[inline]
    pub fn is_universal(&self) -> bool {
        self.component_reads.is_all_set()
            && self.component_writes.is_all_set()
            && self.resource_reads.is_all_set()
            && self.resource_writes.is_all_set()
    }

    /// Constructs an `Access` covering every component and every resource for
    /// both reads and writes. Used by `ExclusiveFunctionSystem` (plan §5.2 /
    /// §12.3) and `ApplyDeferred` (plan §8) to mark systems that must run
    /// alone.
    ///
    /// See [`is_universal`](Access::is_universal) for the rationale on events
    /// being outside the conflict graph.
    #[inline]
    pub fn universal() -> Self {
        let mut access = Self::new();
        access.component_reads.set_all();
        access.component_writes.set_all();
        access.resource_reads.set_all();
        access.resource_writes.set_all();
        access
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

    /// A freshly-constructed `Access` reports no reads/writes anywhere, so it
    /// must not be flagged as universal. Phase 9 §12.5 / §13.1 test
    /// `is_universal_empty_false`.
    #[test]
    fn access_default_is_not_universal() {
        let a = Access::new();
        assert!(!a.is_universal(), "empty access must not be universal");
        let b = Access::default();
        assert!(!b.is_universal(), "Default::default() must match new()");
    }

    /// `Access::universal()` flips every bit across all four bitmasks, so the
    /// predicate must report `true`. Phase 9 §12.5 / §13.1 test
    /// `is_universal_full_true`.
    #[test]
    fn access_universal_is_universal() {
        let a = Access::universal();
        assert!(a.is_universal(), "Access::universal() must satisfy is_universal()");
    }

    /// Setting only a strict subset of bits must NOT trigger `is_universal`.
    /// Phase 9 §12.5 / §13.1 test `is_universal_partial_false`. Covers all
    /// four bitmasks one at a time (component R/W, resource R/W).
    #[test]
    fn access_partial_is_not_universal() {
        let mut only_comp_read = Access::new();
        only_comp_read.add_component_read(ComponentId(0));
        assert!(!only_comp_read.is_universal());

        let mut only_comp_write = Access::new();
        only_comp_write.add_component_write(ComponentId(7));
        assert!(!only_comp_write.is_universal());

        let mut only_res_read = Access::new();
        only_res_read.add_resource_read(ResourceId(0));
        assert!(!only_res_read.is_universal());

        let mut only_res_write = Access::new();
        only_res_write.add_resource_write(ResourceId(255));
        assert!(!only_res_write.is_universal());

        // Three of four bitmasks filled, one bit short: still not universal.
        let mut almost = Access::universal();
        almost.component_writes.unset(ComponentId(42));
        assert!(!almost.is_universal(), "one missing bit must drop universality");
    }

    /// Universal access shares every read/write bit with any non-empty other
    /// access, so `conflicts_with` must fire in both directions. This is the
    /// graph property the scheduler relies on to serialize exclusive systems
    /// (plan §2.5 EXC1).
    #[test]
    fn access_universal_conflicts_with_any_other() {
        let universal = Access::universal();

        // Other declares a single resource read — must conflict (universal
        // writes that resource).
        let mut other_res_read = Access::new();
        other_res_read.add_resource_read(ResourceId(13));
        assert!(universal.conflicts_with(&other_res_read));
        assert!(other_res_read.conflicts_with(&universal));

        // Other declares a single component write — must conflict (universal
        // writes that component too).
        let mut other_comp_write = Access::new();
        other_comp_write.add_component_write(ComponentId(3));
        assert!(universal.conflicts_with(&other_comp_write));
        assert!(other_comp_write.conflicts_with(&universal));

        // Other is itself universal — must conflict (write-vs-write).
        let other_universal = Access::universal();
        assert!(universal.conflicts_with(&other_universal));
    }

    /// `conflicts_with` requires the *other* side to declare at least one
    /// bit; against an empty access, no read/write intersection exists and
    /// the check returns `false`. The scheduler still serializes the empty
    /// system against the universal one through the dependency graph, not
    /// through `Access` alone — see plan §2.5 EXC2.
    #[test]
    fn access_universal_does_not_conflict_with_empty() {
        let universal = Access::universal();
        let empty = Access::new();
        assert!(
            !universal.conflicts_with(&empty),
            "no bits on the other side means no intersection"
        );
        assert!(
            !empty.conflicts_with(&universal),
            "conflict relation is symmetric for the empty case"
        );
    }
}
