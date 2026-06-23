//! Relation-aware query DSL — the QUERY side of the generic Relations API.
//!
//! Three families, all built on the EXISTING relationship storage (the FK
//! [`Relationship`](crate::ecs::core::relationship::Relationship) component on
//! the source + the reverse
//! [`RelationshipTarget`](crate::ecs::core::relationship::RelationshipTarget)
//! collection on the target) — NO side index, NO `HashMap` (Principle 0):
//!
//! * [`traverse_iter`] — transitive / wildcard iterators
//!   ([`TargetsIter`], [`SourcesIter`], [`AncestorsIter`],
//!   [`DescendantsIter`]). Built purely on `get_component` +
//!   `RelationshipTarget::collection()`; NO unsafe. Reusable by the later
//!   relation-aware observer-broadcast phase. Exposed through the
//!   `EcsMaster::{targets, sources, ancestors, descendants}` accessors.
//! * [`filter`] — relation filters
//!   ([`HasRelation`], [`NoRelation`], [`RelatedTo`]) implementing
//!   [`QueryFilter`](crate::ecs::core::iters::query::filter::QueryFilter).
//! * [`related`] — the join term
//!   [`Related<R, D>`] implementing
//!   [`QueryData`](crate::ecs::core::iters::query::data::QueryData).
//!   Read-only join (`D: ReadOnlyQueryData`); yields `Option<D::Item>` per
//!   source row, gathering `D` from the FK target's row. Sequential-only
//!   (const-rejected on `par_iter`).

pub mod filter;
pub mod related;
pub mod traverse_iter;

pub use filter::{HasRelation, NoRelation, RelatedTo};
pub use related::Related;
pub use traverse_iter::{AncestorsIter, DescendantsIter, SourcesIter, TargetsIter};
