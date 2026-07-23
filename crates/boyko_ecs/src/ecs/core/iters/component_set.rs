use std::any::TypeId;
use std::sync::OnceLock;

use boyko_utils::type_intern::TypeIntern;

use crate::ecs::core::component::component::Component;
use crate::ecs::identifiers::primitives::ComponentId;

/// Maximum number of component types in the ECS (mirrors `MAX_COMPONENTS`
/// in `component_registry`).
const MAX_COMPONENTS: usize = 512;

// Global cache for single-component slices indexed by `ComponentId`.
//
// Each slot holds a `&'static [ComponentId]` of length 1 once initialized.
// Using the `ComponentId` (a `usize` in 0..MAX_COMPONENTS) as the index gives
// O(1) lock-free reads after the first call for each component type, with no
// per-call allocation on the warm path.
static SINGLE_COMPONENT_CACHE: [OnceLock<&'static [ComponentId]>; MAX_COMPONENTS] =
    [const { OnceLock::new() }; MAX_COMPONENTS];

/// Hard cap on distinct tuple `ComponentSet` types per process.
///
/// Bounded by the app's distinct arity-2..8 query shapes (tens to low hundreds in practice).
const MAX_TUPLE_SETS: usize = 256;

/// Dense-index intern for tuple `ComponentSet` impls (arity 2-8).
///
/// Rust does NOT create per-monomorphization statics for generic fn bodies; a `static` inside
/// `impl<A, B> Trait for (A, B)` is shared across all `(A, B)` instantiations (rust#22991), so
/// the per-type cache must be keyed on `TypeId::of::<Self>()`.
///
/// 2026-07 audit: the key-to-slice map used to be a `OnceLock<RwLock<HashMap<..>>>` whose
/// documented "cold path … query construction happens at setup" claim did not survive tracing —
/// `cache.read()` ran on EVERY call, before any memo hit was known, so the memo lookup WAS the
/// lock (the `trigger.rs` fingerprint). The single-component leg three lines up
/// ([`SINGLE_COMPONENT_CACHE`]) already had the right shape; the tuple leg now matches it, with
/// [`TypeIntern`] supplying the dense index that `ComponentId` supplies for the single case.
static TUPLE_IDS: TypeIntern<TypeId, { MAX_TUPLE_SETS * 2 }> = TypeIntern::new();

/// Leaked per-tuple-type slices, indexed by [`TUPLE_IDS`].
///
/// Mirrors [`SINGLE_COMPONENT_CACHE`]: one lock-free `OnceLock` slot per interned type, so a
/// warm read is an acquire load and the `Box::leak` happens at most once per type.
static TUPLE_SLICES: [OnceLock<&'static [ComponentId]>; MAX_TUPLE_SETS] =
    [const { OnceLock::new() }; MAX_TUPLE_SETS];

/// Retrieve or initialize the cached `&'static [ComponentId]` for type `T`.
///
/// `init` is called at most once per `T`; subsequent calls return the same pointer with no
/// allocation and no lock — a hash plus two acquire loads.
#[inline]
fn tuple_cache_get_or_init<T: 'static>(
    init: impl FnOnce() -> Vec<ComponentId>,
) -> &'static [ComponentId] {
    let idx = TUPLE_IDS
        .get_or_mint(TypeId::of::<T>())
        .unwrap_or_else(tuple_intern_full) as usize;
    TUPLE_SLICES[idx].get_or_init(|| Box::leak(init().into_boxed_slice()))
}

/// Terminal panic for a full [`TUPLE_IDS`] table.
#[cold]
#[inline(never)]
fn tuple_intern_full() -> u32 {
    panic!(
        "tuple ComponentSet intern full: more than MAX_TUPLE_SETS = {MAX_TUPLE_SETS} distinct \
         arity-2..8 component-set types registered. Raise MAX_TUPLE_SETS (and TUPLE_IDS with it, \
         which must stay at twice the cap) or consolidate query shapes."
    );
}

/// Trait for type-safe component queries.
///
/// Each implementation returns a `&'static [ComponentId]` cached on the first
/// call per distinct component-set type. Subsequent calls return the cached
/// static slice with no allocation.
///
/// # Caching strategy
///
/// - `()` returns `&[]` directly (no heap, no cache).
/// - Single-component impls (`impl<A: Component> ComponentSet for A`) cache
///   a length-1 slice in `SINGLE_COMPONENT_CACHE` indexed by `ComponentId` —
///   lock-free `OnceLock` per slot.
/// - Tuple impls (arity 2-8) cache via a `TypeId`-keyed `RwLock<HashMap>`
///   (cold path: Query/QueryState construction). One `Box::leak` per distinct
///   tuple type in the program's lifetime; total leak <1 KB for any realistic
///   app (~50 distinct tuple types x ~64 bytes = 3 KB worst case).
///
/// # Pointer stability
///
/// All non-empty calls return the same pointer for the same type across all
/// calls. This is critical for any downstream that uses pointer identity
/// (e.g., interning).
pub trait ComponentSet {
    /// Returns the component IDs for all types in this set.
    ///
    /// The returned slice is `'static` and pointer-stable across calls for
    /// the same type.
    fn component_ids() -> &'static [ComponentId];
}

// Implementation for empty tuple — `&[]` is a zero-length static slice requiring
// no heap.
impl ComponentSet for () {
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        &[]
    }
}

// Implementation for single component type.
//
// Uses `SINGLE_COMPONENT_CACHE[A::component_id()]` for a lock-free, per-type
// cache. Each `ComponentId` (0..MAX_COMPONENTS) maps to exactly one `OnceLock`
// slot. The cached pointer is stable across all calls for the same `A`.
impl<A: Component> ComponentSet for A {
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        let id = A::component_id();
        debug_assert!(
            id.0 < MAX_COMPONENTS,
            "ComponentId {id} out of cache range (MAX_COMPONENTS = {MAX_COMPONENTS})"
        );
        // Index by the component's own ID — unique per concrete type by the
        // component registry invariant (C-003).
        SINGLE_COMPONENT_CACHE[id.0].get_or_init(|| Box::leak(vec![id].into_boxed_slice()))
    }
}

// Tuple impls 2-8.
//
// All 7 arities delegate to `tuple_cache_get_or_init::<Self>(...)`, keyed by
// `TypeId::of::<Self>()`. Each distinct tuple type caches exactly one
// `Box::leak`-ed slice on first call; all subsequent calls return the same
// `'static` pointer with no allocation.

/// Implementation for tuple of 2 component types.
impl<A: Component, B: Component> ComponentSet for (A, B) {
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| vec![A::component_id(), B::component_id()])
    }
}

/// Implementation for tuple of 3 component types.
impl<A: Component, B: Component, C: Component> ComponentSet for (A, B, C) {
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![A::component_id(), B::component_id(), C::component_id()]
        })
    }
}

/// Implementation for tuple of 4 component types.
impl<A: Component, B: Component, C: Component, D: Component> ComponentSet for (A, B, C, D) {
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![
                A::component_id(),
                B::component_id(),
                C::component_id(),
                D::component_id(),
            ]
        })
    }
}

/// Implementation for tuple of 5 component types.
impl<A: Component, B: Component, C: Component, D: Component, E: Component> ComponentSet
    for (A, B, C, D, E)
{
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![
                A::component_id(),
                B::component_id(),
                C::component_id(),
                D::component_id(),
                E::component_id(),
            ]
        })
    }
}

/// Implementation for tuple of 6 component types.
impl<A: Component, B: Component, C: Component, D: Component, E: Component, F: Component>
    ComponentSet for (A, B, C, D, E, F)
{
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![
                A::component_id(),
                B::component_id(),
                C::component_id(),
                D::component_id(),
                E::component_id(),
                F::component_id(),
            ]
        })
    }
}

/// Implementation for tuple of 7 component types.
impl<
        A: Component,
        B: Component,
        C: Component,
        D: Component,
        E: Component,
        F: Component,
        G: Component,
    > ComponentSet for (A, B, C, D, E, F, G)
{
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![
                A::component_id(),
                B::component_id(),
                C::component_id(),
                D::component_id(),
                E::component_id(),
                F::component_id(),
                G::component_id(),
            ]
        })
    }
}

/// Implementation for tuple of 8 component types.
impl<
        A: Component,
        B: Component,
        C: Component,
        D: Component,
        E: Component,
        F: Component,
        G: Component,
        H: Component,
    > ComponentSet for (A, B, C, D, E, F, G, H)
{
    #[inline]
    fn component_ids() -> &'static [ComponentId] {
        tuple_cache_get_or_init::<Self>(|| {
            vec![
                A::component_id(),
                B::component_id(),
                C::component_id(),
                D::component_id(),
                E::component_id(),
                F::component_id(),
                G::component_id(),
                H::component_id(),
            ]
        })
    }
}

#[cfg(test)]
mod tests {
    //! Test components use ID range 495-499 (per roadmap test-isolation convention).
    //! Verified free via `grep -rn 'register_layout::<' crates/` — confirmed no
    //! other test uses 495-499 (490-493 used by query_state, 470-471 by query_iter
    //! bench, 480-481 by swap_remove, 450-465 by component_registry).

    use super::*;
    use crate::ecs::core::component::component_registry;

    const ID_A: ComponentId = ComponentId(495);
    const ID_B: ComponentId = ComponentId(496);
    const ID_C: ComponentId = ComponentId(497);
    // 498, 499 reserved for future tests in this module.

    // Minimal #[repr(C)] structs used as stand-ins for real components.
    // They are registered manually via register_layout so no proc-macro is needed.
    #[repr(C)]
    struct TestC495 {
        _x: u32,
    }
    #[repr(C)]
    struct TestC496 {
        _x: u32,
    }
    #[repr(C)]
    struct TestC497 {
        _x: u32,
    }

    // Registers all test component layouts. OnceLock inside register_layout makes
    // this idempotent: calling it from multiple tests in parallel is safe.
    fn register_test_components() {
        component_registry::register_layout::<TestC495>(ID_A.0);
        component_registry::register_layout::<TestC496>(ID_B.0);
        component_registry::register_layout::<TestC497>(ID_C.0);
    }

    // Minimal Component impls: only component_id() is required; all other
    // methods have default bodies in the trait.
    impl Component for TestC495 {
        fn component_id() -> ComponentId { ID_A }
    }
    impl Component for TestC496 {
        fn component_id() -> ComponentId { ID_B }
    }
    impl Component for TestC497 {
        fn component_id() -> ComponentId { ID_C }
    }

    #[test]
    fn t495_empty_tuple_is_empty_slice() {
        let ids = <()>::component_ids();
        assert!(ids.is_empty());
    }

    #[test]
    fn t496_single_component_slice_correct() {
        register_test_components();
        let ids = TestC495::component_ids();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], ID_A);
    }

    #[test]
    fn t497_tuple_slice_correct() {
        register_test_components();
        let ids = <(TestC495, TestC496)>::component_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&ID_A));
        assert!(ids.contains(&ID_B));
    }

    #[test]
    fn t498_static_pointer_stable_across_calls() {
        register_test_components();
        // Pointer stability is guaranteed for single-component types via
        // SINGLE_COMPONENT_CACHE (indexed by ComponentId).
        let p1 = TestC495::component_ids().as_ptr();
        let p2 = TestC495::component_ids().as_ptr();
        assert_eq!(
            p1, p2,
            "SINGLE_COMPONENT_CACHE must return the same pointer on every call for the same type"
        );
    }

    #[test]
    fn t499_distinct_components_distinct_cache_slots() {
        register_test_components();
        // For single-component types, SINGLE_COMPONENT_CACHE uses ComponentId
        // as the array index: IDs 495 and 496 map to different slots, so the
        // returned pointers must differ.
        let p_a = TestC495::component_ids().as_ptr();
        let p_b = TestC496::component_ids().as_ptr();
        assert_ne!(p_a, p_b, "distinct component IDs must map to distinct cache slots");

        // Correctness check: each single-component call returns the right ID.
        assert_eq!(TestC495::component_ids(), &[ID_A]);
        assert_eq!(TestC496::component_ids(), &[ID_B]);
        assert_eq!(TestC497::component_ids(), &[ID_C]);

        // Tuple correctness: each tuple returns the right IDs in correct count.
        assert_eq!(<(TestC495, TestC496)>::component_ids().len(), 2);
        assert_eq!(<(TestC495, TestC496, TestC497)>::component_ids().len(), 3);
    }

    /// Regression test for B1: tuple `component_ids()` must return the same pointer
    /// on every call for the same tuple type. Catches any reversion to
    /// `Box::leak`-per-call behaviour.
    #[test]
    fn t_tuple_pointer_stable_across_calls() {
        register_test_components();
        let p1 = <(TestC495, TestC496)>::component_ids().as_ptr();
        let p2 = <(TestC495, TestC496)>::component_ids().as_ptr();
        assert_eq!(
            p1, p2,
            "TUPLE_CACHE must return the same pointer on every call for the same tuple type"
        );
    }

    /// Distinct tuple types must occupy distinct cache entries (and therefore
    /// distinct pointers), because each has a unique `TypeId`.
    #[test]
    fn t_distinct_tuples_distinct_pointers() {
        register_test_components();
        let p_ab = <(TestC495, TestC496)>::component_ids().as_ptr();
        let p_ac = <(TestC495, TestC497)>::component_ids().as_ptr();
        assert_ne!(
            p_ab, p_ac,
            "distinct tuple types must have distinct TUPLE_CACHE entries"
        );
    }
}
