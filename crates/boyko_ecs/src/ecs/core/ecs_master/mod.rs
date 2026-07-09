// Module name mirrors the public `EcsMaster` type; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod ecs_master;
mod tag_api;
mod enable_tag_api;

// Topic-grouped halves of the `EcsMaster` inherent `impl`, split out of the
// former god-object `ecs_master.rs` (pure mechanical move — inherent-impl
// methods keep their exact `EcsMaster::foo` paths regardless of file).
mod bundle_api;
mod component_api;
mod entity_api;
mod entity_query_api;
mod event_api;
mod observer_api;
mod relationship_api;
mod resource_api;
mod state_api;
mod system_api;
//pub mod archetype_bundle;