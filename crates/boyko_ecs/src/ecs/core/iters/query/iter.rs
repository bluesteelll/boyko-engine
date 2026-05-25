//! `QueryIter` / `QueryIterMut` — hot-path Query iterators.
//!
//! Step 1 reserves the module slot only. The full iterators (per §7 of the
//! Phase 8b plan) with the const-folded archetypal short-circuit and the M2
//! `set_table_readonly` / `set_table_mut` split land in Step 7.

// TODO(phase-8b/step-7): define `QueryIter<'w, 's, D, F>` (read path) and
// `QueryIterMut<'w, 's, D, F>` (write path). Both walk
// `QueryDataState::matched_ids`, resolve archetypes via `archetype_ptr`(_mut),
// invoke `set_table_readonly` or `set_table_mut` per M2, and yield items via
// `D::fetch`. The archetypal short-circuit lives behind `if const {
// F::IS_ARCHETYPAL }`.
