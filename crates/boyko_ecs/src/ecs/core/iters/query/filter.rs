//! `QueryFilter` trait — per-archetype and per-row filtering for queries.
//!
//! Step 1 lands trait + marker-struct definitions only. The full impl (per §5
//! of the Phase 8b plan) — including `()` / `With<C>` / `Without<C>` real
//! bodies, the variadic AND-tuple expansion, and `Or<F>` — arrives in Steps 3
//! and 4.

use std::marker::PhantomData;

// TODO(phase-8b/step-3): replace trait stub with the full `QueryFilter`
// definition per §5.1: `Fetch` GAT, `State`, `IS_ARCHETYPAL`,
// `aggregate_include` / `aggregate_exclude`, `matches_component_set`,
// `init_state`, `set_table_readonly`, `set_table_mut`, `filter_fetch`.
#[allow(dead_code, reason = "Step 1 skeleton; trait body filled in Step 3.")]
/// Marker trait for query-filter types (`With<C>`, `Without<C>`, `Or<...>`,
/// tuples, and the unit `()`).
///
/// The full surface — archetype-level predicate, per-row predicate, access
/// declaration — is added in Step 3.
pub trait QueryFilter: 'static {}

/// `()` is the no-op filter used as the default `F` parameter on
/// [`super::Query`]. Step 3 attaches the real method bodies; the impl is
/// present at Step 1 only so `Query<D>` (with the default filter) type-checks.
impl QueryFilter for () {}

/// Archetype-level inclusion filter: matches archetypes containing component
/// `C`.
///
/// Behaviour and impls land in Step 3.
#[allow(dead_code, reason = "Step 1 skeleton; methods + impl land in Step 3.")]
pub struct With<C> {
    _marker: PhantomData<fn() -> C>,
}

/// Archetype-level exclusion filter: matches archetypes that do NOT contain
/// component `C`.
///
/// Behaviour and impls land in Step 3.
#[allow(dead_code, reason = "Step 1 skeleton; methods + impl land in Step 3.")]
pub struct Without<C> {
    _marker: PhantomData<fn() -> C>,
}

// `Or<F>` (post-filter OR-combinator) is introduced in Step 4 alongside the
// variadic tuple AND impls.
