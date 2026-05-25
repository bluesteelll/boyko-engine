//! Per-system metadata: name, declared access, observed generations.
//!
//! See Phase 8a plan §4.2 / §9 for the field rationale.
//!
//! # Layout sizing (Q5 RESOLUTION)
//!
//! `Access` (192 B) + `&'static str` (16 B fat pointer) + 2 × `ArchetypeGeneration`
//! (`NonZeroUsize`, 8 B each = 16 B) = **224 B**. The struct spans 4 cache
//! lines with no internal padding under natural alignment.

use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::system::access::Access;

/// Per-system context carried alongside the system body.
///
/// Holds the declared [`Access`] surface, a diagnostic name, and the
/// archetype generations observed at the last refresh (consumed by
/// Phase 8b `Query::new_archetype`).
///
/// `SystemMeta` is constructed empty via [`SystemMeta::new`] and filled
/// during the system's two-phase init (`SystemParam::init_state` then
/// `SystemParam::init_access`). After [`FilteredAccessSet::finalize`] copies
/// the accumulated [`Access`] into the meta, the structure is read-only for
/// the rest of the system's lifetime.
///
/// [`FilteredAccessSet::finalize`]: super::FilteredAccessSet::finalize
#[repr(C)]
pub struct SystemMeta {
    /// Read/write surface declared by the system's parameters.
    ///
    /// Filled during `init_access` via the [`FilteredAccessSet`] accumulator;
    /// read-only thereafter (write-once contract — see [`Access`] docs).
    ///
    /// [`FilteredAccessSet`]: super::FilteredAccessSet
    pub(crate) access: Access,

    /// Diagnostic name (`std::any::type_name::<Self>()` by default).
    pub(crate) name: &'static str,

    /// Last `archetype_generation` observed at the last `new_archetype` /
    /// `init_access` pass. Phase 8b `Query::new_archetype` uses this to
    /// decide when to refresh archetype caches.
    pub(crate) last_archetype_generation: ArchetypeGeneration,

    /// Last `structural_generation` observed. Same pattern; consumed by
    /// Phase 8b dual-generation cache to detect ArchetypeId-ABA hazards.
    pub(crate) last_structural_generation: ArchetypeGeneration,
}

impl SystemMeta {
    /// Constructs a fresh meta with the given diagnostic `name` and empty
    /// [`Access`].
    ///
    /// Both generation fields start at [`ArchetypeGeneration::FIRST`] — the
    /// canonical "never observed" sentinel that compares less than any
    /// post-bump value the master can produce. Phase 8b `Query` overwrites
    /// these on its first archetype-refresh pass.
    pub fn new(name: &'static str) -> Self {
        Self {
            access: Access::new(),
            name,
            last_archetype_generation: ArchetypeGeneration::FIRST,
            last_structural_generation: ArchetypeGeneration::FIRST,
        }
    }

    /// Returns the diagnostic name (typically `std::any::type_name::<F>()`
    /// of the underlying function or closure).
    #[inline]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the declared read/write surface.
    ///
    /// Phase 9 scheduler calls this on every system to build the conflict
    /// graph; Phase 8a uses it for diagnostics and end-to-end assertions.
    #[inline]
    pub fn access(&self) -> &Access {
        &self.access
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SystemMeta::new` produces a meta whose `Access` carries no reads or
    /// writes.
    #[test]
    fn new_initialises_with_empty_access() {
        let meta = SystemMeta::new("test_system");
        let other = Access::new();
        // Two empty accesses must not conflict — neither has any bits set.
        assert!(
            !meta.access().conflicts_with(&other),
            "freshly-constructed SystemMeta must have empty Access"
        );
    }

    /// `SystemMeta::name` returns the `&'static str` passed at construction.
    #[test]
    fn name_returns_static_str() {
        let meta = SystemMeta::new("alpha_system");
        assert_eq!(meta.name(), "alpha_system");
    }

    /// Generation fields start at `ArchetypeGeneration::FIRST`.
    #[test]
    fn generations_start_at_first() {
        let meta = SystemMeta::new("gen_test");
        assert_eq!(meta.last_archetype_generation, ArchetypeGeneration::FIRST);
        assert_eq!(meta.last_structural_generation, ArchetypeGeneration::FIRST);
    }
}
