pub mod component_mask;
// Module name mirrors the public `Component` trait; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod component;
pub mod component_registry;
pub mod component_pool_bundle;
