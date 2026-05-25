//! `QueryData` trait — typed component access for queries.
//!
//! Step 1 lands trait definitions only; the full impl (per §4 of the Phase 8b
//! plan) — including `&T` / `&mut T` impls, the `set_table_readonly` /
//! `set_table_mut` split (M2), and variadic tuple support — arrives in Steps
//! 2 and 4.

// TODO(phase-8b/step-2): replace trait stubs with the full `QueryData`
// definition per §4.1: `Fetch` GAT, `State`, `IS_READ_ONLY`, `init_state`,
// `init_access`, `set_table_readonly`, `set_table_mut`, `fetch`,
// `matches_component_set`.
#[allow(dead_code, reason = "Step 1 skeleton; trait body filled in Step 2.")]
/// Marker trait for query-data types (e.g. `&T`, `&mut T`, tuples).
///
/// The full surface — fetch GAT, access declaration, post-filter predicate —
/// is added in Step 2 of the Phase 8b roll-out.
pub trait QueryData: 'static {}

#[allow(dead_code, reason = "Step 1 skeleton; populated in Step 2.")]
/// Sub-trait of [`QueryData`] for read-only data (`&T`, tuples of `&T`).
///
/// `Query::iter()` is gated on this bound so that mutable data forces callers
/// to use `iter_mut()`.
pub trait ReadOnlyQueryData: QueryData {}

/// Maximum tuple arity supported by [`QueryData`] variadic impls.
///
/// Tuples beyond this arity trip a `const { panic!() }` in Step 4 — the limit
/// keeps macro expansion bounded and the I-cache budget honest.
pub const MAX_QUERY_DATA_ARITY: usize = 12;
