// Module name mirrors the public `EcsMaster` type; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod ecs_master;
mod tag_api;
mod enable_tag_api;
//pub mod archetype_bundle;