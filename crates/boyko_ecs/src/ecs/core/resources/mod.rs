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

pub mod resource;
pub(crate) mod resource_registry;
// `resources.rs` (slab storage) is added in Phase 8a Step 2.

pub use resource::Resource;
pub use resource_registry::RESOURCE_SLOT_COUNT;
