use crate::ecs::core::component::component::Component;
use crate::ecs::identifiers::primitives::ComponentId;

/// Trait for type-safe component queries
/// Implemented for tuples of component types
pub trait ComponentSet {
    /// Returns the component IDs for all types in the set
    fn component_ids() -> Vec<ComponentId>;
}

// Implementation for empty tuple (useful for base case)
impl ComponentSet for () {
    fn component_ids() -> Vec<ComponentId> {
        Vec::new()
    }
}

// Implementation for single component type
impl<A: Component> ComponentSet for A {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id()]
    }
}

// Implementation for tuple of 2 component types
impl<A: Component, B: Component> ComponentSet for (A, B) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id()]
    }
}

// Implementation for tuple of 3 component types
impl<A: Component, B: Component, C: Component> ComponentSet for (A, B, C) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id()]
    }
}

// Implementation for tuple of 4 component types
impl<A: Component, B: Component, C: Component, D: Component> ComponentSet for (A, B, C, D) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id(), D::component_id()]
    }
}

// Implementation for tuple of 5 component types
impl<A: Component, B: Component, C: Component, D: Component, E: Component> ComponentSet for (A, B, C, D, E) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id(), D::component_id(), E::component_id()]
    }
}

// Implementation for tuple of 6 component types
impl<A: Component, B: Component, C: Component, D: Component, E: Component, F: Component> ComponentSet for (A, B, C, D, E, F) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id(), D::component_id(), E::component_id(), F::component_id()]
    }
}

// Implementation for tuple of 7 component types
impl<A: Component, B: Component, C: Component, D: Component, E: Component, F: Component, G: Component> ComponentSet for (A, B, C, D, E, F, G) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id(), D::component_id(), E::component_id(), F::component_id(), G::component_id()]
    }
}

// Implementation for tuple of 8 component types
impl<A: Component, B: Component, C: Component, D: Component, E: Component, F: Component, G: Component, H: Component> ComponentSet for (A, B, C, D, E, F, G, H) {
    fn component_ids() -> Vec<ComponentId> {
        vec![A::component_id(), B::component_id(), C::component_id(), D::component_id(), E::component_id(), F::component_id(), G::component_id(), H::component_id()]
    }
} 