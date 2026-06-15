pub mod component_mask;
// Module name mirrors the public `Component` trait; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod component;
pub mod component_registry;
pub mod component_pool_bundle;
pub mod hooks;
pub mod observers;
// Wave-1 foundation: the paged enable-bit storage + presence oracle land ahead
// of their consumers (archetype wiring in Wave 2, toggle API + migration in
// Wave 3). Until those waves wire the call sites, the storage surface is
// legitimately unused; the allow is removed when the consumers land.
#[allow(dead_code)]
pub(crate) mod enable;
