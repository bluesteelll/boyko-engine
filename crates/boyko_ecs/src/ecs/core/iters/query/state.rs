//! `QueryDataState<D, F>` — per-system Query state cache.
//!
//! Step 1 reserves the module slot only. The full struct + dual-invariant
//! assertion (M1) lands in Step 6 of the Phase 8b plan.
//!
//! See §6 of `docs/PHASE-8B-QUERY-DSL-PLAN.md` for the design.

// TODO(phase-8b/step-6): define `QueryDataState<D: QueryData, F: QueryFilter>`
// with `matched_ids`, `matched_archetypes`, `last_observed_*`, `data_state`,
// `filter_state`. Implement `new`, `update`, `init_access`,
// `post_filter_matched`, and the debug-only `assert_dual_invariant`.
