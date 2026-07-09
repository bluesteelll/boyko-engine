//! Relation-aware traversal accessors on [`EcsMaster`] (mechanical split).
//!
//! `targets` / `sources` / `ancestors` / `descendants` over relationship
//! storage. Extracted verbatim from `ecs_master.rs`.

use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── Relation-aware traversal accessors ──────────────────────────────────
    //
    // Thin wrappers over the transitive/wildcard iterators in
    // `iters::query::relation::traverse_iter`. They walk ONLY the existing
    // relationship storage (FK component + reverse collection) through
    // `get_component` — no side index (Principle 0). The transitive walks are
    // depth-capped at `MAX_PROPAGATION_DEPTH`; a non-`ACYCLIC` relation guards
    // revisits with a function-local cold visited set, const-folded away for an
    // acyclic relation (e.g. `ChildOf`).

    /// Iterates the single target of relationship `R` for `source` (cardinality
    /// 0..1): yields the FK target if `source` carries `R`, otherwise nothing.
    #[inline]
    pub fn targets<R: crate::ecs::core::relationship::Relationship>(
        &self,
        source: Entity,
    ) -> crate::ecs::core::iters::query::relation::TargetsIter<'_, R> {
        crate::ecs::core::iters::query::relation::TargetsIter::new(self, source)
    }

    /// Iterates every source whose relationship `R` foreign key points at
    /// `target`, by reading the target's reverse `R::Target` collection (the
    /// existing reverse index — no all-entity scan).
    #[inline]
    pub fn sources<R: crate::ecs::core::relationship::Relationship>(
        &self,
        target: Entity,
    ) -> crate::ecs::core::iters::query::relation::SourcesIter<'_, R> {
        crate::ecs::core::iters::query::relation::SourcesIter::new(self, target)
    }

    /// Walks UP the relationship `R` chain from `e` (`e -> R.target -> ...`),
    /// yielding each ancestor target in turn (NOT `e` itself). Depth-capped;
    /// cycle-safe for a non-`ACYCLIC` relation.
    #[inline]
    pub fn ancestors<R: crate::ecs::core::relationship::Relationship>(
        &self,
        e: Entity,
    ) -> crate::ecs::core::iters::query::relation::AncestorsIter<'_, R> {
        crate::ecs::core::iters::query::relation::AncestorsIter::new(self, e)
    }

    /// Walks DOWN the relationship `R` chain from `root` (DFS over the reverse
    /// `R::Target` collections), yielding each descendant source (NOT `root`
    /// itself). Depth-capped; each node visited at most once for a non-`ACYCLIC`
    /// relation.
    #[inline]
    pub fn descendants<R: crate::ecs::core::relationship::Relationship>(
        &self,
        root: Entity,
    ) -> crate::ecs::core::iters::query::relation::DescendantsIter<'_, R> {
        crate::ecs::core::iters::query::relation::DescendantsIter::new(self, root)
    }

}
