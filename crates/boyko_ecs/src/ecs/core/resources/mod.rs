//! Resource subsystem — world-global singletons addressed by [`ResourceId`].
//!
//! A resource is a 1-instance-per-type value owned by the world. Unlike
//! components (per-entity, dense storage), resources live in a sparse slab
//! addressed by their globally-assigned [`ResourceId`]. Each type may be
//! registered as **either** a `Component` **or** a `Resource` — never both
//! (enforced at registration time; see [`resource_registry::register_new`]).
//!
//! See Phase 8a plan §5.1 for the full design.
//!
//! [`ResourceId`]: crate::ecs::identifiers::primitives::ResourceId

// Phase 4 Seam 2 — non-`Send` resource storage. Parallel of `resources` /
// `resource_registry`, for types that carry no `Send + Sync` bound (RHI
// handles, FFI pointers). See `NonSendResource` (D6 + CR-A).
pub(crate) mod nonsend_resource_registry;
pub mod nonsend_resources;
pub mod resource;
pub(crate) mod resource_registry;
// First-class kernel registry that interns a `TypeId → ResourceId` mapping for
// generic resource types (`State<S>` / `NextState<S>` / `StateTransitionRecord<S>`
// and `boyko_input`'s `ActionState<A>` / `InputMap<A>`), where the per-impl
// `static SLOT` idiom is unsound (rust#22991). Publishing it replaces the
// duplicate registry `boyko_input` previously hand-rolled (Principle 0).
pub mod resource_type_registry;
// Module name mirrors the public `Resources` slab type; the parent module
// `resources` is the subsystem namespace.
#[allow(clippy::module_inception)]
pub mod resources;

pub use nonsend_resources::NonSendResources;
pub use resource::{NonSendResource, Resource};
pub use resource_registry::RESOURCE_SLOT_COUNT;
pub use resources::Resources;

// `register_new` is re-exported through the public `resources` module so
// `#[derive(Resource)]`-generated code in downstream crates can reach it
// without taking a `pub(crate)` direct path on `resource_registry`. The
// macro emits `::boyko_ecs::ecs::core::resources::register_new::<Self>()`
// (see `boyko_macros::resource_macro`); this re-export is the matching
// entry point. Q6 RESOLUTION: keeps the registry module gated while still
// allowing external derive expansion.
pub use resource_registry::register_new;

// `resource_id_for` is the first-class kernel entry point for interning a
// generic resource type's `ResourceId`. `State<S>` and `boyko_input`'s
// `ActionState<A>` / `InputMap<A>` reach it through this re-export. See
// `resource_type_registry` for the rust#22991 rationale.
pub use resource_type_registry::resource_id_for;
