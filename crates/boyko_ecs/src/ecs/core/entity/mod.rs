// Module name mirrors the public `Entity` type; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod entity;
pub mod entity_inland;
pub mod entity_master;