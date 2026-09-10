//! KE13 — the per-row filter predicate for a **point lookup**, shared by
//! [`Query`](super::query::Query) and [`QueryView`](super::query_view::QueryView).
//!
//! `iter()` applies `F` per row through `filter_fetch`; a point lookup must
//! apply the same predicate or the two disagree on the same query. `Query`
//! grew that call when KE3 landed random access; `QueryView` did not, and a
//! dense `With<C>` / `Without<C>` — non-archetypal, with a
//! `matches_component_set` that admits every archetype — was therefore gated by
//! nothing at all on `QueryView::get` / `get_mut` (KE13).
//!
//! The repair is ONE implementation, not a second copy: the body below is
//! `Query::point_filter_passes`'s, moved here verbatim, and both types call it.
//! Copying it was how the defect opened — `QueryView::get` grew the `D`-side
//! `resolve_dense` mirror and not the `F`-side one — so a shared function is
//! the fix and a mirrored one is the defect again.

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Applies `F`'s per-row predicate at `row` — the twin of the `filter_fetch`
/// call `QueryIter::next` makes when it crosses into an archetype.
///
/// Const-folds away entirely for `F::IS_ARCHETYPAL` (every `With`/`Without`
/// over a table component, every `Or` of those, and `()`): the first statement
/// is `return true`, so an archetypal point lookup emits nothing. That fold is
/// why this function is `#[inline]` and must NOT become `#[inline(never)]` —
/// an out-of-line call whose whole body is `return true` would put a real
/// `call` on every `Query::get` in the engine.
///
/// # Safety
///
/// * `arch_ptr` must be a live-for-`'w` archetype pointer that satisfies
///   `F::matches_component_set` — i.e. one the query's matched set named.
/// * `row` must be `< (*arch_ptr).entity_count()`.
/// * `meta` must reference the currently-active system's [`SystemMeta`], and is
///   read ONLY when `F::NEEDS_CHANGE_DETECTION`. A caller with no live system
///   meta must therefore hold `!F::NEEDS_CHANGE_DETECTION`; `QueryView` pins
///   that with a `const` assert at each call site rather than by convention.
#[inline]
pub(super) unsafe fn point_filter_passes<'w, F: QueryFilter>(
    filter_state: &<F as QueryFilter>::State,
    world: UnsafeEcsCell<'w>,
    arch_ptr: *const Archetype,
    row: usize,
    meta: &SystemMeta,
) -> bool {
    if const { F::IS_ARCHETYPAL } {
        // QF1: an archetypal filter's `filter_fetch` is unconditionally `true`;
        // the whole block below vanishes at monomorphisation.
        return true;
    }
    let mut filter_fetch = <F as QueryFilter>::init_fetch(filter_state);
    // Dense plan D3: a dense `With`/`Without` term caches its global
    // `DenseStore` pointer here — the point-lookup twin of the resolve the
    // cursors do in `QueryIter::new`. Without it `filter_fetch` reads a NULL
    // store as an answer (KE1's mechanism, one path over).
    if const { F::HAS_DENSE } {
        // SAFETY (D3): `world` is the cell scoped to `'w`; the resolved store
        //   pointer is address-stable for that lifetime — the SAME cell the
        //   iter path passes to `resolve_dense`.
        unsafe {
            <F as QueryFilter>::resolve_dense(&mut filter_fetch, filter_state, world);
        }
    }
    // NCD6 const-fold dispatcher, mirroring `QueryIter::next`. The `_no_meta`
    // variants are a `#[cold] panic!` for NCD = true impls, so routing must be
    // const-exact. A filter's tick reads are shared regardless of the caller's
    // mutability (`Changed::set_table_mut` delegates to `set_table_readonly`
    // for exactly this reason), so the read-only surface is correct on both
    // `get` and `get_mut`.
    //
    // SAFETY (QF3): `arch_ptr` is live for `'w` and satisfies
    //   `F::matches_component_set` (it is in the matched set); `meta` is the
    //   active system's `SystemMeta` per this function's contract.
    unsafe {
        if const { F::NEEDS_CHANGE_DETECTION } {
            <F as QueryFilter>::set_table_readonly(&mut filter_fetch, filter_state, arch_ptr, meta);
        } else {
            <F as QueryFilter>::set_table_readonly_no_meta(
                &mut filter_fetch,
                filter_state,
                arch_ptr,
            );
        }
    }
    // SAFETY (QF1): the `set_table_*` above initialised `filter_fetch` for this
    //   archetype; `row < entity_count` per this function's contract.
    unsafe { <F as QueryFilter>::filter_fetch(&filter_fetch, row) }
}
