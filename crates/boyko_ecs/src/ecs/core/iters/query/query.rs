//! `Query<'w, 's, D, F>` — typed component query SystemParam.
//!
//! Step 1 reserves the struct shape only. The full SystemParam impl,
//! IntoIterator (C1), and the `iter` / `iter_mut` methods land in Step 8.

use std::marker::PhantomData;

use crate::ecs::core::iters::query::data::QueryData;
use crate::ecs::core::iters::query::filter::QueryFilter;

// TODO(phase-8b/step-8): replace this skeleton with the full struct + impl
// per §3 of the plan:
//   - `archetype_count`, `is_empty`, `iter`, `iter_mut`,
//   - `IntoIterator for &Query<...>` / `for &mut Query<...>` (C1),
//   - `unsafe impl SystemParam for Query<...>` with the C3 binder fix.
#[allow(dead_code, reason = "Step 1 skeleton; fields populated in Step 8.")]
/// Typed component query.
///
/// The lifetime parameters and generic shape are stable from Step 1 so that
/// downstream Steps 2-7 can name `Query<D, F>` in trait impls and docs without
/// further churn.
///
/// - `'w` — world borrow lifetime (the `UnsafeEcsCell` reference).
/// - `'s` — per-system state borrow lifetime (the cached `QueryDataState`).
/// - `D` — [`QueryData`] (e.g. `&T`, `&mut T`, or a tuple thereof).
/// - `F` — [`QueryFilter`] (defaults to `()`, the no-op filter).
pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> {
    _marker_w: PhantomData<&'w ()>,
    _marker_s: PhantomData<&'s ()>,
    _marker_d: PhantomData<fn() -> D>,
    _marker_f: PhantomData<fn() -> F>,
}
