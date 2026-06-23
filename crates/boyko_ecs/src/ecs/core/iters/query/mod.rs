//! Phase 8b typed `Query<D, F>` DSL.
//!
//! Replaces the legacy `iters::LegacyQuery` for system-level use; the legacy
//! path remains available for backward compatibility (see
//! [`crate::ecs::core::iters::legacy_query`]).
//!
//! # Submodules
//!
//! - [`data`] — [`QueryData`] / [`ReadOnlyQueryData`] traits (Step 2).
//! - [`filter`] — [`QueryFilter`] / [`With`] / [`Without`] / `Or` (Steps 3–4).
//! - [`state`] — `QueryDataState<D, F>` per-system cache (Step 6).
//! - [`iter`] — `QueryIter` / `QueryIterMut` hot-path iterators (Step 7).
//! - [`query`] — `Query<'w, 's, D, F>` SystemParam (Step 8).
//!
//! # Current status: Step 1 (skeleton)
//!
//! Only trait names and unit-struct placeholders exist. Method bodies, impls,
//! and tests land in subsequent steps. The skeleton is sufficient to compile
//! against `iters::query::{QueryData, QueryFilter, With, Without, Query}` from
//! callers that will be written in Steps 2+.

pub mod chunk_iter;
pub mod chunked_data;
pub mod data;
// `IsEnabled<T>`: non-filtering, order-preserving per-row read of an EnableTag
// bit (the `bool`-valued twin of `Option<&T>`). Lets a system read an enable
// bit per row WITHOUT dropping/reordering a row (unlike the `Enabled<T>` filter).
pub mod data_is_enabled;
// Dense plan D3 (FORK 2): the opt-in pure-dense fast-path cursors. Kept SEPARATE
// from `iter` so `Query::iter` stays byte-identical (the 0%-gate).
pub mod dense_iter;
// EnableTag Step 9: stack-only dynamic per-row enable terms (the `with_enabled`
// / `without_enabled` builders). Crate-internal carrier; the term ceiling is
// `crate::ecs::constants::MAX_ENABLE_TERMS`.
pub mod enable_terms;
pub mod filter;
// EnableTag Step 7: the `Enabled<T>` / `Disabled<T>` non-archetypal per-row
// filters over the bitset storage backend.
pub mod filter_enable;
pub mod iter;
pub mod par_chunk;
pub mod par_iter;
// The `query` submodule houses the `Query<'w, 's, D, F>` struct itself; the
// inception is intentional and dictated by the Phase 8b plan file layout
// (§17.1). The submodule cannot be folded into `mod.rs` because the file is
// expected to grow to ~300 lines in Step 8 (Query + SystemParam + IntoIterator).
#[allow(clippy::module_inception)]
pub mod query;
pub mod query_type_registry;
// Relation-aware DSL: the JOIN term (`Related<R, D>`), relation filters
// (`HasRelation` / `NoRelation` / `RelatedTo`), and the transitive/wildcard
// traversal iterators (`targets` / `sources` / `ancestors` / `descendants`).
// Built ONLY on the existing relationship FK + reverse-collection storage
// (Principle 0) — no side index.
pub mod relation;
pub mod query_view;
pub mod state;
// Phase 22 D4: stack-only dynamic-tag terms. The `TagTerms` carrier itself is
// crate-internal (terms are added via `with_tag` / `without_tag` builders);
// only the term-count ceiling is part of the public contract.
pub mod tag_terms;
// Phase 22.1 Area A: immutable epoch lists + lock-free CAS publication. The
// SOUNDNESS-CRITICAL term-prefilter memo (protocol P1–P4); crate-internal.
pub mod term_list;

pub use chunked_data::ChunkedQueryData;
pub use data::{AnyOf, Mut, QueryData, ReadOnlyQueryData, Ref};
pub use data_is_enabled::IsEnabled;
pub use dense_iter::{DenseQueryData, DenseQueryIter, DenseQueryIterMut};
pub use filter::{Added, ArchetypalQueryFilter, Changed, Or, QueryFilter, With, Without};
pub use filter_enable::{Disabled, Enabled};
pub use iter::{QueryIter, QueryIterEntities, QueryIterEntitiesMut, QueryIterMut};
pub use par_iter::{BatchingStrategy, MIN_ARCHETYPE_FOR_PARALLEL, ParQuery, ParQueryMut};
pub use query::Query;
pub use query_type_registry::{MAX_QUERY_TYPES, QueryTypeId, QueryTypeKey};
pub use relation::{
    AncestorsIter, DescendantsIter, HasRelation, NoRelation, Related, RelatedTo, SourcesIter,
    TargetsIter,
};
pub use query_view::QueryView;
pub use state::QueryDataState;
pub use tag_terms::MAX_DYN_TAG_TERMS;
// `Or` is re-exported when Step 4 lands its definition.
