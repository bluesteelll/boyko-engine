//! Ordering primitives consumed by [`ScheduleBuilder`].
//!
//! See Phase 9 plan §5.8 for the `OrderingEdge` enum and §5.5 for the
//! `SystemKey` builder handle. Wave 4 Step 9 ships the raw edge
//! representation; the builder rewrites these into post-topological-sort
//! `SystemIndex` edges during [`ScheduleBuilder::build`].
//!
//! # `SystemKey` vs `SystemIndex`
//!
//! `SystemKey` is the **pre-build** handle returned by `add_system` —
//! stable for the lifetime of the builder, opaque to the user. The
//! eventual `SystemIndex` (a `u16` newtype in `conflict_graph`) is
//! assigned by Kahn's topological sort during `build`. Keeping the two
//! kinds nominally distinct prevents a user from accidentally passing
//! a post-build index into `.before(...)` / `.after(...)`.
//!
//! [`ScheduleBuilder`]: super::schedule_builder::ScheduleBuilder
//! [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build

use crate::ecs::core::schedule::system_set::SystemSetId;

/// Opaque pre-build handle for a system added to a [`ScheduleBuilder`].
///
/// The wrapped `usize` is the system's index inside
/// `ScheduleBuilder::descriptors`. The newtype is `Copy` so chaining
/// `.before(other).after(another)` does not consume the receiver-side
/// handle; equality and hashing are derived for use as map keys in the
/// builder's set-expansion phase (Wave 5 Step 14).
///
/// [`ScheduleBuilder`]: super::schedule_builder::ScheduleBuilder
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct SystemKey(pub usize);

/// Raw ordering relation collected by the builder during
/// `add_system`/`SystemConfig` chaining.
///
/// The variants are turned into DAG edges by `expand_ordering_edges`
/// inside [`ScheduleBuilder::build`]:
///
/// * `Before(a, b)` → DAG edge `a → b` (a must complete before b starts).
/// * `After(a, b)` → DAG edge `b → a` (same semantic; producer-side hint).
/// * `ChainConsecutive(a, b)` → DAG edge `a → b` (identical to `Before`
///   at the graph layer; preserved as a separate variant so the
///   diagnostics in `cycle_in_before_after_panics` can name the
///   originating builder call).
/// * `InSet(a, set)` → consumed by Wave 5 Step 14's set-expansion pass
///   (membership turns into pairwise edges between the set's members and
///   anything ordered relative to the set). Wave 4 Step 9 records the
///   membership without expanding it; the conflict-graph build treats
///   in-set hints as zero ordering constraints.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // Wave 4 Step 9 — `InSet` consumed by Step 14 expansion.
pub(crate) enum OrderingEdge {
    /// `Before(a, b)` — system `a` must finish before `b` starts.
    Before(SystemKey, SystemKey),

    /// `After(a, b)` — equivalent to `Before(b, a)` from the receiver's
    /// perspective. Preserved as a separate variant so diagnostics can
    /// quote the builder call the user wrote.
    After(SystemKey, SystemKey),

    /// `ChainConsecutive(a, b)` — emitted by `SystemConfig::chain` to
    /// enforce a strict serial order between two systems. Same DAG
    /// edge as `Before` but separate variant for diagnostics.
    ChainConsecutive(SystemKey, SystemKey),

    /// `InSet(a, set)` — system `a` belongs to the named set. The
    /// builder records this without expanding it; set-membership ↔ edge
    /// translation lands with the sync-point analyzer in Wave 5 Step 14.
    InSet(SystemKey, SystemSetId),
}

impl OrderingEdge {
    /// Returns the DAG predecessor/successor pair (`from → to`) if this
    /// edge contributes one. `InSet` returns `None`.
    ///
    /// Used by [`ScheduleBuilder::build`] when constructing the ordering
    /// DAG and by `tarjan_scc` when scanning for cycles.
    ///
    /// [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build
    #[inline]
    pub(crate) fn as_dag_edge(&self) -> Option<(SystemKey, SystemKey)> {
        match *self {
            OrderingEdge::Before(a, b) => Some((a, b)),
            OrderingEdge::After(a, b) => Some((b, a)),
            OrderingEdge::ChainConsecutive(a, b) => Some((a, b)),
            OrderingEdge::InSet(_, _) => None,
        }
    }
}

/// Set-level ordering relation, collected on the **builder** (not on a
/// per-system descriptor — a set has no single descriptor to own it).
///
/// Phase 15 §3.1. Expanded into `(SystemKey, SystemKey)` pairs by
/// `expand_set_edges` during `ScheduleBuilder::build`, after the set
/// hierarchy is flattened (D3) so `members(S)` is the transitive leaf
/// membership. `SystemBeforeSet`/`SystemAfterSet` capture system↔set
/// ordering (`SystemConfig::before_set`/`after_set`); `SetBeforeSet`
/// captures set↔set ordering (`ConfigureSet::before`/`after`).
//
// The shared `Set` suffix is intentional and load-bearing: it marks the
// **target** of each relation as a set (vs the system↔system `OrderingEdge`).
// The variant names match the authoritative Phase 15 plan §3.1 verbatim;
// renaming for the lint would obscure the system↔set vs set↔set distinction.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum SetOrderEdge {
    /// System `X` runs before every member of set `S`.
    SystemBeforeSet(SystemKey, SystemSetId),
    /// System `X` runs after every member of set `S`.
    SystemAfterSet(SystemKey, SystemSetId),
    /// Every member of set `S` runs before every member of set `T`.
    /// Covers `configure_set(S).before(T)` and `configure_set(T).after(S)`.
    SetBeforeSet(SystemSetId, SystemSetId),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Before(a, b)` and `ChainConsecutive(a, b)` produce identical DAG
    /// edges; `After(a, b)` flips the direction.
    #[test]
    fn as_dag_edge_directions() {
        let a = SystemKey(0);
        let b = SystemKey(1);
        assert_eq!(OrderingEdge::Before(a, b).as_dag_edge(), Some((a, b)));
        assert_eq!(OrderingEdge::After(a, b).as_dag_edge(), Some((b, a)));
        assert_eq!(
            OrderingEdge::ChainConsecutive(a, b).as_dag_edge(),
            Some((a, b))
        );
    }

    /// `InSet` contributes no DAG edge by itself — set expansion (Step 14)
    /// turns membership into pairwise edges.
    #[test]
    fn in_set_contributes_no_edge() {
        let a = SystemKey(0);
        let set = SystemSetId(0);
        assert_eq!(OrderingEdge::InSet(a, set).as_dag_edge(), None);
    }
}
