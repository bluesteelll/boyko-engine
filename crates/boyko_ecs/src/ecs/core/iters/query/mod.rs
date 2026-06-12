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
pub mod filter;
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
pub mod query_view;
pub mod state;
// Phase 22 D4: stack-only dynamic-tag terms. The `TagTerms` carrier itself is
// crate-internal (terms are added via `with_tag` / `without_tag` builders);
// only the term-count ceiling is part of the public contract.
pub mod tag_terms;

pub use chunked_data::ChunkedQueryData;
pub use data::{Mut, QueryData, ReadOnlyQueryData, Ref};
pub use filter::{Added, ArchetypalQueryFilter, Changed, Or, QueryFilter, With, Without};
pub use iter::{QueryIter, QueryIterMut};
pub use par_iter::{BatchingStrategy, MIN_ARCHETYPE_FOR_PARALLEL, ParQuery, ParQueryMut};
pub use query::Query;
pub use query_type_registry::{MAX_QUERY_TYPES, QueryTypeId, QueryTypeKey};
pub use query_view::QueryView;
pub use state::QueryDataState;
pub use tag_terms::MAX_DYN_TAG_TERMS;
// `Or` is re-exported when Step 4 lands its definition.
